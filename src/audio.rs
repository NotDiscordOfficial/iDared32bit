/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Audio file decoding and OpenAL bindings.
//!
//! The audio file decoding support is an abstraction over various libraries
//! (currently [hound] and [symphonia]), usage of which should be
//! confined to this module.
//!
//! Resources:
//! - [Apple Core Audio Format Specification 1.0](https://developer.apple.com/library/archive/documentation/MusicAudio/Reference/CAFSpec/CAF_intro/CAF_intro.html)

mod ima4;
pub mod openal;
mod symphonia_formats;

pub use symphonia_formats::{Mp3StreamDecoder, PacketStreamDecoder};

pub use ima4::decode_ima4;

use crate::fs::{Fs, GuestPath};
use std::io::Cursor;

#[derive(Debug)]
pub enum AudioFileOpenError {
    FileReadError,
    FileDecodeError,
}

#[derive(Debug)]
pub enum AudioFormat {
    LinearPcm {
        is_float: bool,
        is_little_endian: bool,
    },
}
/// A compressed format that [AudioFile] decodes to PCM when opening a file.
///
/// Apps still see the file as compressed, as on a real device: its packets
/// are variable-size, each holding [Self::frames_per_packet] frames. Unlike on
/// a real device though, the packets' contents are the decoded 16-bit PCM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompressedFormat {
    Aac,
    Mp3,
    Alac,
}

impl CompressedFormat {
    pub fn frames_per_packet(self) -> u32 {
        match self {
            CompressedFormat::Aac => 1024,
            CompressedFormat::Mp3 => 1152,
            CompressedFormat::Alac => 4096,
        }
    }
}

/// Fields have the same meanings as in the Core Audio Format's
/// Audio Description chunk, which is in turn similar to Core Audio Types'
/// `AudioStreamBasicDescription`.
#[derive(Debug)]
pub struct AudioDescription {
    /// Hz
    pub sample_rate: f64,
    pub format: AudioFormat,
    pub bytes_per_packet: u32,
    pub frames_per_packet: u32,
    pub channels_per_frame: u32,
    pub bits_per_channel: u32,
}

/// The inner file, and the file type as a FourCC (`AudioFileTypeID`), if known.
pub struct AudioFile(AudioFileInner, Option<[u8; 4]>);
enum AudioFileInner {
    Wave(hound::WavReader<Cursor<Vec<u8>>>),
    Symphonia(symphonia_formats::SymphoniaDecodedToPcm),
}

/// Guess a file's type (`AudioFileTypeID`) from its first bytes.
fn sniff_file_type(bytes: &[u8]) -> Option<[u8; 4]> {
    let at = |offset: usize, magic: &[u8]| bytes.get(offset..offset + magic.len()) == Some(magic);
    if at(0, b"RIFF") && at(8, b"WAVE") {
        Some(*b"WAVE")
    } else if at(0, b"caff") {
        Some(*b"caff")
    } else if at(0, b"FORM") && at(8, b"AIFC") {
        Some(*b"AIFC")
    } else if at(0, b"FORM") {
        Some(*b"AIFF")
    } else if at(4, b"ftyp") {
        Some(*b"m4af")
    } else if at(0, b"ID3") || (bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0) {
        // ADTS AAC also starts with 0xfff, but layer bits 00 mean it's not MP3.
        if bytes[1] & 0x06 == 0 && !at(0, b"ID3") {
            Some(*b"adts")
        } else {
            Some(*b"MPG3")
        }
    } else {
        None
    }
}

fn wav_to_pcm16<R: std::io::Read>(
    reader: hound::WavReader<R>,
) -> Result<symphonia_formats::SymphoniaDecodedToPcm, AudioFileOpenError> {
    let spec = reader.spec();
    let samples: Result<Vec<i16>, _> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .map(|s| s.map(|s| (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16))
            .collect(),
        hound::SampleFormat::Int => {
            let shift = spec.bits_per_sample.saturating_sub(16);
            reader
                .into_samples::<i32>()
                .map(|s| s.map(|s| (s >> shift) as i16))
                .collect()
        }
    };
    let samples = samples.map_err(|_| AudioFileOpenError::FileDecodeError)?;
    Ok(symphonia_formats::SymphoniaDecodedToPcm {
        bytes: samples.iter().flat_map(|s| s.to_le_bytes()).collect(),
        sample_rate: spec.sample_rate,
        channels: spec.channels.into(),
        compressed_format: None,
    })
}

/// Symphonia can't read 8-bit linear PCM in CAF files, so convert it to
/// 16-bit here. CAF's integer samples are always signed.
fn caf_pcm8_to_pcm16(bytes: &[u8]) -> Option<symphonia_formats::SymphoniaDecodedToPcm> {
    let u32_at = |at: usize| Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    if bytes.get(..4)? != b"caff" {
        return None;
    }
    let mut desc = None;
    let mut pos = 8;
    while pos + 12 <= bytes.len() {
        let chunk_type = &bytes[pos..pos + 4];
        let size = i64::from_be_bytes(bytes[pos + 4..pos + 12].try_into().unwrap());
        let start = pos + 12;
        // A size of -1 means the chunk (always data) runs to the end.
        let end = if size < 0 {
            bytes.len()
        } else {
            start.checked_add(size.try_into().ok()?)?.min(bytes.len())
        };
        match chunk_type {
            b"desc" => {
                let rate = f64::from_bits(u64::from_be_bytes(
                    bytes.get(start..start + 8)?.try_into().ok()?,
                ));
                let format_id = bytes.get(start + 8..start + 12)?;
                let is_float = u32_at(start + 12)? & 1 != 0;
                let channels = u32_at(start + 24)?;
                let bits = u32_at(start + 28)?;
                if format_id != b"lpcm" || is_float || bits != 8 || channels == 0 {
                    return None;
                }
                desc = Some((rate, channels));
            }
            b"data" => {
                let (rate, channels) = desc?;
                // Skip the edit count.
                let samples = bytes.get(start + 4..end)?;
                return Some(symphonia_formats::SymphoniaDecodedToPcm {
                    bytes: samples
                        .iter()
                        .flat_map(|&s| (i16::from(s as i8) << 8).to_le_bytes())
                        .collect(),
                    sample_rate: rate as u32,
                    channels,
                    compressed_format: None,
                });
            }
            _ => (),
        }
        pos = end;
    }
    None
}

/// Reads linear PCM in AIFF files as 16-bit. Symphonia isn't used as it
/// rejects files iOS plays, such as ones whose SSND chunk is shorter than
/// COMM's frame count says (UConnect's sounds): the frames are read to the
/// end of the file here instead.
fn aiff_to_pcm16(bytes: &[u8]) -> Option<symphonia_formats::SymphoniaDecodedToPcm> {
    let u32_at = |at: usize| Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    if bytes.get(..4)? != b"FORM" || bytes.get(8..12)? != b"AIFF" {
        return None;
    }
    let (mut comm, mut data_start) = (None, None);
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let start = pos + 8;
        match &bytes[pos..pos + 4] {
            b"COMM" => {
                let channels = u16::from_be_bytes(bytes.get(start..start + 2)?.try_into().ok()?);
                let frames = u32_at(start + 2)?;
                let bits = u16::from_be_bytes(bytes.get(start + 6..start + 8)?.try_into().ok()?);
                // 80-bit extended float: 15-bit exponent, 64-bit mantissa.
                let exp = u16::from_be_bytes(bytes.get(start + 8..start + 10)?.try_into().ok()?);
                let mantissa =
                    u64::from_be_bytes(bytes.get(start + 10..start + 18)?.try_into().ok()?);
                let shift = (16383 + 63u32).checked_sub(u32::from(exp & 0x7fff))?;
                let rate = mantissa.checked_shr(shift)?;
                if channels == 0 || !(1..=32).contains(&bits) {
                    return None;
                }
                comm = Some((channels, frames, bits, rate));
            }
            b"SSND" => data_start = Some(start + 8 + usize::try_from(u32_at(start)?).ok()?),
            _ => (),
        }
        // Chunks are padded to an even size.
        let size = usize::try_from(u32_at(pos + 4)?).ok()?;
        pos = start.checked_add(size)?.checked_add(size & 1)?;
    }
    let (channels, frames, bits, rate) = comm?;
    let width = usize::from(bits).div_ceil(8);
    let len = usize::try_from(frames).ok()? * usize::from(channels) * width;
    let samples = bytes.get(data_start?..)?;
    let samples = &samples[..len.min(samples.len() / width * width)];
    Some(symphonia_formats::SymphoniaDecodedToPcm {
        // Samples are signed and big-endian: keep the top 16 bits.
        bytes: samples
            .chunks(width)
            .flat_map(|s| [s.get(1).copied().unwrap_or(0), s[0]])
            .collect(),
        sample_rate: rate.try_into().ok()?,
        channels: channels.into(),
        compressed_format: None,
    })
}

impl AudioFile {
    pub fn open_for_reading<P: AsRef<GuestPath>>(
        path: P,
        fs: &Fs,
    ) -> Result<Self, AudioFileOpenError> {
        // TODO: it would be better not to load the whole file at once
        let Ok(bytes) = fs.read(path.as_ref()) else {
            // TODO: Handle other FS related errors?
            return Err(AudioFileOpenError::FileReadError);
        };

        if let Ok(bytes) = Self::read_from_vec(bytes) {
            Ok(bytes)
        } else {
            log!(
                "Could not decode audio file at path {:?}, likely an unimplemented file format.",
                path.as_ref()
            );
            Err(AudioFileOpenError::FileReadError)
        }
    }

    pub fn read_from_vec(bytes: Vec<u8>) -> Result<Self, AudioFileOpenError> {
        let file_type = sniff_file_type(&bytes);
        Self::read_from_vec_inner(bytes).map(|inner| AudioFile(inner, file_type))
    }

    fn read_from_vec_inner(bytes: Vec<u8>) -> Result<AudioFileInner, AudioFileOpenError> {
        // WavReader::new() consume the reader (in this case, a Cursor)
        // passed to it. This is a bit annoying considering we don't know
        // which is appropriate for the file. This is worked around here by
        // using temporary readers for checking if the file is the supported
        // format, then recreating the reader if that works.
        if let Ok(reader) = hound::WavReader::new(Cursor::new(&bytes)) {
            let spec = reader.spec();
            if spec.sample_format == hound::SampleFormat::Int
                && matches!(spec.bits_per_sample, 8 | 16)
            {
                let reader = hound::WavReader::new(Cursor::new(bytes)).unwrap();
                Ok(AudioFileInner::Wave(reader))
            } else {
                // Other sample formats are converted to 16-bit up front.
                Ok(AudioFileInner::Symphonia(wav_to_pcm16(reader)?))
            }
        // TODO: Real MP3/MP4/Non-linear PCM container handling. Currently we
        // are immediately decoding the entire file to PCM and acting as if
        // it's a PCM file, simply because because this is easier. Full MP3
        // support would require a lot of changes in Audio Toolbox.
        } else if let Some(pcm) = aiff_to_pcm16(&bytes) {
            Ok(AudioFileInner::Symphonia(pcm))
        } else if let Some(pcm) = caf_pcm8_to_pcm16(&bytes) {
            Ok(AudioFileInner::Symphonia(pcm))
        } else if let Ok(pcm) = symphonia_formats::decode_symphonia_to_pcm(Cursor::new(bytes)) {
            Ok(AudioFileInner::Symphonia(pcm))
        } else {
            Err(AudioFileOpenError::FileDecodeError)
        }
    }

    pub fn audio_description(&self) -> AudioDescription {
        match self.0 {
            AudioFileInner::Wave(ref wave_reader) => {
                let hound::WavSpec {
                    channels,
                    sample_rate,
                    bits_per_sample,
                    sample_format,
                } = wave_reader.spec();
                // Other formats were converted in read_from_vec().
                assert!(matches!(bits_per_sample, 8 | 16));
                assert!(sample_format == hound::SampleFormat::Int);

                AudioDescription {
                    sample_rate: sample_rate.into(),
                    format: AudioFormat::LinearPcm {
                        is_float: false,
                        is_little_endian: true,
                    },
                    bytes_per_packet: u32::from(channels * bits_per_sample / 8),
                    frames_per_packet: 1,
                    channels_per_frame: channels.into(),
                    bits_per_channel: bits_per_sample as u32,
                }
            }
            AudioFileInner::Symphonia(symphonia_formats::SymphoniaDecodedToPcm {
                sample_rate,
                channels,
                ..
            }) => AudioDescription {
                sample_rate: f64::from(sample_rate),
                format: AudioFormat::LinearPcm {
                    is_float: false,
                    is_little_endian: true,
                },
                bytes_per_packet: channels * 2,
                frames_per_packet: 1,
                channels_per_frame: channels,
                bits_per_channel: 16,
            },
        }
    }

    /// The file type as a FourCC (`AudioFileTypeID`), if known.
    pub fn file_type(&self) -> Option<[u8; 4]> {
        self.1
    }

    /// The compressed format the file was decoded from, if any.
    pub fn compressed_format(&self) -> Option<CompressedFormat> {
        match self.0 {
            AudioFileInner::Wave(_) => None,
            AudioFileInner::Symphonia(ref pcm) => pcm.compressed_format,
        }
    }

    /// For a file decoded from a compressed format: the size of a packet as
    /// apps see it (see [CompressedFormat]). The last packet may be shorter.
    pub fn compressed_packet_size(&self) -> Option<u32> {
        let format = self.compressed_format()?;
        let AudioDescription {
            bytes_per_packet, ..
        } = self.audio_description();
        Some(format.frames_per_packet() * bytes_per_packet)
    }

    /// For a file decoded from a compressed format: the number of packets as
    /// apps see them (see [CompressedFormat]).
    pub fn compressed_packet_count(&self) -> Option<u64> {
        let packet_size = u64::from(self.compressed_packet_size()?);
        Some(self.byte_count().div_ceil(packet_size))
    }

    /// For a file decoded from a compressed format: the byte offsets and sizes
    /// of up to `max_packets` packets from `start_packet`, stopping before the
    /// total size would exceed `max_bytes`, if given.
    pub fn compressed_packets(
        &self,
        start_packet: u64,
        max_packets: u32,
        max_bytes: Option<u32>,
    ) -> Vec<(u64, u32)> {
        let packet_size = self.compressed_packet_size().unwrap();
        let byte_count = self.byte_count();
        let mut packets = Vec::new();
        let mut total: u32 = 0;
        for i in 0..u64::from(max_packets) {
            let offset = (start_packet + i) * u64::from(packet_size);
            if offset >= byte_count {
                break;
            }
            let size = u64::from(packet_size).min(byte_count - offset) as u32;
            if max_bytes.is_some_and(|max_bytes| total + size > max_bytes) {
                break;
            }
            packets.push((offset, size));
            total += size;
        }
        packets
    }

    fn bytes_per_sample(&self) -> u64 {
        let AudioDescription {
            format,
            bytes_per_packet,
            frames_per_packet,
            channels_per_frame,
            ..
        } = self.audio_description();
        if !matches!(format, AudioFormat::LinearPcm { .. }) {
            panic!("{format:?} is a compressed format!");
        }
        ((bytes_per_packet / frames_per_packet) / channels_per_frame).into()
    }

    pub fn byte_count(&self) -> u64 {
        match self.0 {
            AudioFileInner::Wave(ref wave_reader) => {
                let sample_count = wave_reader.len(); // position-independent
                u64::from(sample_count) * self.bytes_per_sample()
            }
            AudioFileInner::Symphonia(symphonia_formats::SymphoniaDecodedToPcm {
                ref bytes,
                ..
            }) => bytes.len() as u64,
        }
    }

    pub fn packet_count(&self) -> u64 {
        match self.0 {
            AudioFileInner::Wave(_)
            | AudioFileInner::Symphonia(symphonia_formats::SymphoniaDecodedToPcm { .. }) => {
                // never variable-size
                self.byte_count() / u64::from(self.packet_size_fixed())
            }
        }
    }

    /// Returns the packet size if this audio format has a constant packet size,
    /// panics if not.
    pub fn packet_size_fixed(&self) -> u32 {
        let AudioDescription {
            bytes_per_packet, ..
        } = self.audio_description();
        assert!(bytes_per_packet != 0);
        bytes_per_packet
    }

    pub fn packet_size_upper_bound(&self) -> u32 {
        self.packet_size_fixed() // variable size not implemented
    }

    /// Read `buffer.len()` bytes of audio data from byte offset `offset`.
    /// Returns the number of bytes read.
    pub fn read_bytes(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize, ()> {
        match self.0 {
            AudioFileInner::Wave(_) => {
                let bytes_per_sample = self.bytes_per_sample();
                assert!(offset.is_multiple_of(bytes_per_sample));
                assert!(u64::try_from(buffer.len())
                    .unwrap()
                    .is_multiple_of(bytes_per_sample));

                let sample_count = u64::try_from(buffer.len()).unwrap() / bytes_per_sample;
                let sample_count: usize = sample_count.try_into().unwrap();

                let AudioFileInner::Wave(ref mut wave_reader) = self.0 else {
                    unreachable!()
                };

                let channels: u64 = wave_reader.spec().channels.into();
                // WavReader expects number of samples which are
                // independent of the number of channels here
                wave_reader
                    .seek((offset / (bytes_per_sample * channels)).try_into().unwrap())
                    .map_err(|_| ())?;

                let mut byte_offset = 0;
                for sample in wave_reader.samples().take(sample_count) {
                    let sample: i16 = sample.map_err(|_| ())?;
                    match bytes_per_sample {
                        // From the OpenAL docs: 8-bit PCM data is expressed as
                        // an unsigned value over the range 0 to 255, 128 being
                        // an audio output level of zero. Loaded wav samples
                        // must be converted to that from signed with 0 as
                        // output level 0.
                        1 => buffer[byte_offset] = (sample + 128) as u8,
                        2 => buffer[byte_offset..][..2].copy_from_slice(&sample.to_le_bytes()),
                        _ => todo!(),
                    }
                    byte_offset += bytes_per_sample as usize;
                }
                Ok(byte_offset)
            }
            AudioFileInner::Symphonia(symphonia_formats::SymphoniaDecodedToPcm {
                ref bytes,
                ..
            }) => {
                let bytes = bytes.get(offset as usize..).ok_or(())?;
                let bytes_to_read = buffer.len().min(bytes.len());
                let bytes = &bytes[..bytes_to_read];
                buffer[..bytes_to_read].copy_from_slice(bytes);
                Ok(bytes_to_read)
            }
        }
    }

    pub fn estimated_duration(&self) -> f64 {
        let AudioDescription {
            sample_rate,
            bytes_per_packet,
            frames_per_packet,
            ..
        } = self.audio_description();
        assert!(bytes_per_packet != 0); // TODO
        self.byte_count() as f64 * frames_per_packet as f64
            / (bytes_per_packet as f64 * sample_rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_types() {
        assert_eq!(sniff_file_type(b"RIFF\0\0\0\0WAVEfmt "), Some(*b"WAVE"));
        assert_eq!(sniff_file_type(b"caff\0\x01\0\0"), Some(*b"caff"));
        assert_eq!(sniff_file_type(b"FORM\0\0\0\0AIFF"), Some(*b"AIFF"));
        assert_eq!(sniff_file_type(b"FORM\0\0\0\0AIFC"), Some(*b"AIFC"));
        assert_eq!(sniff_file_type(b"\0\0\0\x20ftypM4A "), Some(*b"m4af"));
        assert_eq!(sniff_file_type(b"ID3\x03\0"), Some(*b"MPG3"));
        assert_eq!(sniff_file_type(&[0xff, 0xfb, 0x90, 0x00]), Some(*b"MPG3"));
        assert_eq!(sniff_file_type(&[0xff, 0xf1, 0x50, 0x80]), Some(*b"adts"));
        assert_eq!(sniff_file_type(b"OggS"), None);
        assert_eq!(sniff_file_type(b""), None);
    }

    #[test]
    fn compressed_packets() {
        // 2.5 AAC packets of 16-bit stereo
        let file = AudioFile(
            AudioFileInner::Symphonia(symphonia_formats::SymphoniaDecodedToPcm {
                bytes: vec![0; 2560 * 4],
                sample_rate: 44100,
                channels: 2,
                compressed_format: Some(CompressedFormat::Aac),
            }),
            None,
        );
        assert_eq!(file.compressed_packet_count(), Some(3));
        assert_eq!(
            file.compressed_packets(0, 10, None),
            vec![(0, 4096), (4096, 4096), (8192, 2048)]
        );
        assert_eq!(file.compressed_packets(0, 10, Some(8191)), vec![(0, 4096)]);
    }

    #[test]
    fn wav_24_bit_is_read_as_16_bit() {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 22050,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        };
        let mut bytes = Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(&mut bytes, spec).unwrap();
        writer.write_sample(0x123456).unwrap();
        writer.finalize().unwrap();

        let mut file = AudioFile::read_from_vec(bytes.into_inner()).unwrap();
        let mut sample = [0; 2];
        assert_eq!(file.read_bytes(0, &mut sample), Ok(2));
        assert_eq!(i16::from_le_bytes(sample), 0x1234);
    }
}
