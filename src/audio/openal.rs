/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! A pure-Rust implementation of the subset of OpenAL 1.1 touchHLE uses,
//! rendering audio through SDL. Replaces the old OpenAL Soft C library
//! wrapper; the public API ([OpenAL], [OpenALContext], [OpenALManager],
//! `al_types`/`alc_types`, `AL_*`/`ALC_*`) is kept close to the old shape so
//! callers need minimal changes.
//!
//! SDL only gives a raw output stream, so mixing happens here:
//! [MixerCallback] sums all playing sources across all contexts, resampling
//! and applying gain/panning/attenuation/Doppler.
//!
//! [OpenAL 1.1 specification](https://www.openal.org/documentation/openal-1.1-specification.pdf)

// The public API mirrors OpenAL's C API, which is not snake-case.
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
// Full OpenAL constant set defined for completeness; not all are used yet.
#![allow(dead_code)]

use sdl2::audio::{AudioCallback, AudioDevice, AudioSpec, AudioSpecDesired};
use sdl2::AudioSubsystem;
use std::collections::{HashMap, VecDeque};
use std::ffi;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

/// OpenAL scalar types. These mirror the C `<AL/al.h>` typedefs.
#[allow(dead_code)]
pub mod al_types {
    use std::ffi;

    pub type ALboolean = ffi::c_char;
    pub type ALchar = ffi::c_char;
    pub type ALbyte = ffi::c_schar;
    pub type ALubyte = ffi::c_uchar;
    pub type ALshort = ffi::c_short;
    pub type ALushort = ffi::c_ushort;
    pub type ALint = ffi::c_int;
    pub type ALuint = ffi::c_uint;
    pub type ALsizei = ffi::c_int;
    pub type ALenum = ffi::c_int;
    pub type ALfloat = ffi::c_float;
    pub type ALdouble = ffi::c_double;
    pub type ALvoid = ffi::c_void;
}

/// OpenAL context types. These mirror the C `<AL/alc.h>` typedefs.
#[allow(dead_code)]
pub mod alc_types {
    use std::ffi;

    /// Opaque type. touchHLE treats devices as opaque tokens.
    pub type ALCdevice = ffi::c_void;
    /// Opaque type. touchHLE treats contexts as opaque tokens.
    pub type ALCcontext = ffi::c_void;

    pub type ALCboolean = ffi::c_char;
    pub type ALCchar = ffi::c_char;
    pub type ALCint = ffi::c_int;
    pub type ALCuint = ffi::c_uint;
    pub type ALCsizei = ffi::c_int;
    pub type ALCenum = ffi::c_int;
}

use al_types::*;
use alc_types::{ALCboolean, ALCchar, ALCenum, ALCint};

// === Constants (values from the OpenAL 1.1 specification) ===

pub const AL_NONE: ALenum = 0;
pub const AL_FALSE: ALboolean = 0;
pub const AL_TRUE: ALboolean = 1;

pub const AL_SOURCE_RELATIVE: ALenum = 0x202;
pub const AL_CONE_INNER_ANGLE: ALenum = 0x1001;
pub const AL_CONE_OUTER_ANGLE: ALenum = 0x1002;
pub const AL_PITCH: ALenum = 0x1003;
pub const AL_POSITION: ALenum = 0x1004;
pub const AL_DIRECTION: ALenum = 0x1005;
pub const AL_VELOCITY: ALenum = 0x1006;
pub const AL_LOOPING: ALenum = 0x1007;
pub const AL_BUFFER: ALenum = 0x1009;
pub const AL_GAIN: ALenum = 0x100A;
pub const AL_MIN_GAIN: ALenum = 0x100D;
pub const AL_MAX_GAIN: ALenum = 0x100E;
pub const AL_ORIENTATION: ALenum = 0x100F;
pub const AL_SOURCE_STATE: ALenum = 0x1010;
pub const AL_INITIAL: ALenum = 0x1011;
pub const AL_PLAYING: ALenum = 0x1012;
pub const AL_PAUSED: ALenum = 0x1013;
pub const AL_STOPPED: ALenum = 0x1014;
pub const AL_BUFFERS_QUEUED: ALenum = 0x1015;
pub const AL_BUFFERS_PROCESSED: ALenum = 0x1016;
pub const AL_REFERENCE_DISTANCE: ALenum = 0x1020;
pub const AL_ROLLOFF_FACTOR: ALenum = 0x1021;
pub const AL_CONE_OUTER_GAIN: ALenum = 0x1022;
pub const AL_MAX_DISTANCE: ALenum = 0x1023;
pub const AL_SEC_OFFSET: ALenum = 0x1024;
pub const AL_SAMPLE_OFFSET: ALenum = 0x1025;
pub const AL_BYTE_OFFSET: ALenum = 0x1026;
pub const AL_SOURCE_TYPE: ALenum = 0x1027;
pub const AL_STATIC: ALenum = 0x1028;
pub const AL_STREAMING: ALenum = 0x1029;
pub const AL_UNDETERMINED: ALenum = 0x1030;

pub const AL_FORMAT_MONO8: ALenum = 0x1100;
pub const AL_FORMAT_MONO16: ALenum = 0x1101;
pub const AL_FORMAT_STEREO8: ALenum = 0x1102;
pub const AL_FORMAT_STEREO16: ALenum = 0x1103;

pub const AL_FREQUENCY: ALenum = 0x2001;
pub const AL_BITS: ALenum = 0x2002;
pub const AL_CHANNELS: ALenum = 0x2003;
pub const AL_SIZE: ALenum = 0x2004;

pub const AL_NO_ERROR: ALenum = 0;
pub const AL_INVALID_NAME: ALenum = 0xA001u32 as ALenum;
pub const AL_INVALID_ENUM: ALenum = 0xA002u32 as ALenum;
pub const AL_INVALID_VALUE: ALenum = 0xA003u32 as ALenum;
pub const AL_INVALID_OPERATION: ALenum = 0xA004u32 as ALenum;
pub const AL_OUT_OF_MEMORY: ALenum = 0xA005u32 as ALenum;

pub const AL_VENDOR: ALenum = 0xB001u32 as ALenum;
pub const AL_VERSION: ALenum = 0xB002u32 as ALenum;
pub const AL_RENDERER: ALenum = 0xB003u32 as ALenum;
pub const AL_EXTENSIONS: ALenum = 0xB004u32 as ALenum;

pub const AL_DOPPLER_FACTOR: ALenum = 0xC000u32 as ALenum;
pub const AL_DOPPLER_VELOCITY: ALenum = 0xC001u32 as ALenum;
pub const AL_SPEED_OF_SOUND: ALenum = 0xC003u32 as ALenum;
pub const AL_DISTANCE_MODEL: ALenum = 0xD000u32 as ALenum;

pub const AL_INVERSE_DISTANCE: ALenum = 0xD001u32 as ALenum;
pub const AL_INVERSE_DISTANCE_CLAMPED: ALenum = 0xD002u32 as ALenum;
pub const AL_LINEAR_DISTANCE: ALenum = 0xD003u32 as ALenum;
pub const AL_LINEAR_DISTANCE_CLAMPED: ALenum = 0xD004u32 as ALenum;
pub const AL_EXPONENT_DISTANCE: ALenum = 0xD005u32 as ALenum;
pub const AL_EXPONENT_DISTANCE_CLAMPED: ALenum = 0xD006u32 as ALenum;

pub const ALC_FALSE: ALCboolean = 0;
pub const ALC_TRUE: ALCboolean = 1;
pub const ALC_FREQUENCY: ALCint = 0x1007;
pub const ALC_REFRESH: ALCint = 0x1008;
pub const ALC_SYNC: ALCint = 0x1009;
pub const ALC_MONO_SOURCES: ALCint = 0x1010;
pub const ALC_STEREO_SOURCES: ALCint = 0x1011;

pub const ALC_NO_ERROR: ALCenum = 0;
pub const ALC_INVALID_DEVICE: ALCenum = 0xA001u32 as ALCenum;
pub const ALC_INVALID_CONTEXT: ALCenum = 0xA002u32 as ALCenum;
pub const ALC_INVALID_ENUM: ALCenum = 0xA003u32 as ALCenum;
pub const ALC_INVALID_VALUE: ALCenum = 0xA004u32 as ALCenum;
pub const ALC_OUT_OF_MEMORY: ALCenum = 0xA005u32 as ALCenum;

pub const ALC_DEFAULT_DEVICE_SPECIFIER: ALCenum = 0x1004;
pub const ALC_DEVICE_SPECIFIER: ALCenum = 0x1005;
pub const ALC_EXTENSIONS: ALCenum = 0x1006;

/// Name reported for the (single, default) audio device.
const DEVICE_SPECIFIER: &[u8] = b"touchHLE SDL audio\0";

// === Mixer state (shared between guest and SDL audio threads) ===

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceState {
    Initial,
    Playing,
    Paused,
    Stopped,
}

/// A decoded PCM buffer. Samples are stored as interleaved `f32` in `[-1, 1]`.
struct Buffer {
    samples: Vec<f32>,
    channels: u32,
    sample_rate: u32,
    /// Original bit depth, retained only for `alGetBufferi(AL_BITS)`.
    bits: u32,
}
impl Buffer {
    fn empty() -> Self {
        Buffer {
            samples: Vec::new(),
            channels: 1,
            sample_rate: 44100,
            bits: 16,
        }
    }
    fn frame_count(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / self.channels as usize
        }
    }
}

struct Source {
    state: SourceState,
    gain: f32,
    min_gain: f32,
    max_gain: f32,
    pitch: f32,
    looping: bool,
    source_relative: bool,
    position: [f32; 3],
    velocity: [f32; 3],
    direction: [f32; 3],
    reference_distance: f32,
    max_distance: f32,
    rolloff_factor: f32,
    /// Whether the source is playing a single statically-attached buffer
    /// (`AL_BUFFER`) rather than a streaming queue.
    is_static: bool,
    /// Buffer name set via `alSourcei(AL_BUFFER, ...)`, or 0 for none.
    static_buffer: ALuint,
    /// Playlist of buffer names. For a static source this is `[static_buffer]`.
    queue: VecDeque<ALuint>,
    /// Index into `queue` of the buffer currently being played.
    current: usize,
    /// Fractional frame position within the current buffer.
    sample_pos: f64,
    /// Number of buffers consumed and available to be unqueued (streaming).
    processed: u32,
}
impl Default for Source {
    fn default() -> Self {
        Source {
            state: SourceState::Initial,
            gain: 1.0,
            min_gain: 0.0,
            max_gain: 1.0,
            pitch: 1.0,
            looping: false,
            source_relative: false,
            position: [0.0; 3],
            velocity: [0.0; 3],
            direction: [0.0; 3],
            reference_distance: 1.0,
            max_distance: f32::MAX,
            rolloff_factor: 1.0,
            is_static: false,
            static_buffer: 0,
            queue: VecDeque::new(),
            current: 0,
            sample_pos: 0.0,
            processed: 0,
        }
    }
}
impl Source {
    fn al_state(&self) -> ALint {
        match self.state {
            SourceState::Initial => AL_INITIAL,
            SourceState::Playing => AL_PLAYING,
            SourceState::Paused => AL_PAUSED,
            SourceState::Stopped => AL_STOPPED,
        }
    }

    /// Start (or restart) playback. Resuming from pause keeps the cursor.
    fn play(&mut self) {
        if self.state == SourceState::Paused {
            self.state = SourceState::Playing;
            return;
        }
        self.current = 0;
        self.sample_pos = 0.0;
        if !self.is_static {
            self.processed = 0;
        }
        self.state = SourceState::Playing;
    }

    fn pause(&mut self) {
        if self.state == SourceState::Playing {
            self.state = SourceState::Paused;
        }
    }

    /// Stop playback. Per the spec, stopping marks all queued buffers as
    /// processed so they can be unqueued.
    fn stop(&mut self) {
        self.state = SourceState::Stopped;
        if !self.is_static {
            self.processed = self.queue.len() as u32;
            self.current = self.queue.len();
        }
    }

    fn rewind(&mut self) {
        self.state = SourceState::Initial;
        self.current = 0;
        self.sample_pos = 0.0;
        if !self.is_static {
            self.processed = 0;
        }
    }

    /// Advance to the next buffer in the playlist, updating counters and
    /// handling looping / underrun.
    fn advance_buffer(&mut self) {
        self.current += 1;
        if !self.is_static {
            self.processed = self.processed.saturating_add(1);
        }
        if self.current >= self.queue.len() {
            if self.looping && !self.queue.is_empty() {
                self.current = 0;
                self.sample_pos = 0.0;
                if !self.is_static {
                    self.processed = 0;
                }
            } else {
                self.state = SourceState::Stopped;
            }
        }
    }

    fn world_position(&self, listener: &Listener) -> [f32; 3] {
        if self.source_relative {
            [
                listener.position[0] + self.position[0],
                listener.position[1] + self.position[1],
                listener.position[2] + self.position[2],
            ]
        } else {
            self.position
        }
    }

    fn distance_gain(&self, model: ALenum, listener: &Listener) -> f32 {
        let rel = if self.source_relative {
            self.position
        } else {
            [
                self.position[0] - listener.position[0],
                self.position[1] - listener.position[1],
                self.position[2] - listener.position[2],
            ]
        };
        let dist = (rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2]).sqrt();
        let dref = self.reference_distance;
        let dmax = self.max_distance;
        let rolloff = self.rolloff_factor;

        // max() then min() rather than clamp(), which panics if a game sets
        // the maximum below the minimum: then the maximum wins.
        let g = match model {
            AL_NONE => 1.0,
            AL_INVERSE_DISTANCE | AL_INVERSE_DISTANCE_CLAMPED => {
                let d = if model == AL_INVERSE_DISTANCE_CLAMPED {
                    dist.max(dref).min(dmax)
                } else {
                    dist.max(dref)
                };
                let denom = dref + rolloff * (d - dref);
                if denom.abs() < 1e-9 {
                    1.0
                } else {
                    dref / denom
                }
            }
            AL_LINEAR_DISTANCE | AL_LINEAR_DISTANCE_CLAMPED => {
                let d = if model == AL_LINEAR_DISTANCE_CLAMPED {
                    dist.max(dref).min(dmax)
                } else {
                    dist.min(dmax)
                };
                let span = dmax - dref;
                if span.abs() < 1e-9 {
                    1.0
                } else {
                    1.0 - rolloff * (d - dref) / span
                }
            }
            AL_EXPONENT_DISTANCE | AL_EXPONENT_DISTANCE_CLAMPED => {
                let d = if model == AL_EXPONENT_DISTANCE_CLAMPED {
                    dist.max(dref).min(dmax)
                } else {
                    dist.max(1e-6)
                };
                if dref <= 0.0 {
                    1.0
                } else {
                    (d / dref).powf(-rolloff)
                }
            }
            _ => 1.0,
        };
        if g.is_finite() {
            g.clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    /// Constant-power stereo panning gains for a mono source.
    fn pan_gains(&self, listener: &Listener) -> (f32, f32) {
        let rel = if self.source_relative {
            self.position
        } else {
            [
                self.position[0] - listener.position[0],
                self.position[1] - listener.position[1],
                self.position[2] - listener.position[2],
            ]
        };
        let mag = (rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2]).sqrt();
        if mag < 1e-6 {
            return (
                std::f32::consts::FRAC_1_SQRT_2,
                std::f32::consts::FRAC_1_SQRT_2,
            );
        }
        let pan = (rel[0] / mag).clamp(-1.0, 1.0);
        let angle = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
        (angle.cos(), angle.sin())
    }

    fn doppler_pitch(&self, listener: &Listener, factor: f32, speed_of_sound: f32) -> f32 {
        if factor == 0.0 || speed_of_sound == 0.0 {
            return 1.0;
        }
        let src_pos = self.world_position(listener);
        let sl = [
            listener.position[0] - src_pos[0],
            listener.position[1] - src_pos[1],
            listener.position[2] - src_pos[2],
        ];
        let dist = (sl[0] * sl[0] + sl[1] * sl[1] + sl[2] * sl[2]).sqrt();
        if dist < 1e-6 {
            return 1.0;
        }
        let unit = [sl[0] / dist, sl[1] / dist, sl[2] / dist];
        let dot = |a: &[f32; 3], b: &[f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let cap = speed_of_sound / factor;
        let vls = dot(&listener.velocity, &unit).min(cap);
        let vss = dot(&self.velocity, &unit).min(cap);
        let num = speed_of_sound - factor * vls;
        let den = speed_of_sound - factor * vss;
        if den.abs() < 1e-6 {
            return 1.0;
        }
        let p = num / den;
        if p.is_finite() {
            p.clamp(0.25, 4.0)
        } else {
            1.0
        }
    }

    /// Render this source's contribution into the interleaved output buffer,
    /// advancing its playback cursor.
    #[allow(clippy::too_many_arguments)]
    fn render(
        &mut self,
        out: &mut [f32],
        frames: usize,
        channels: usize,
        buffers: &HashMap<ALuint, Buffer>,
        device_rate: f64,
        model: ALenum,
        doppler_factor: f32,
        speed_of_sound: f32,
        listener: &Listener,
    ) {
        if self.state != SourceState::Playing || device_rate <= 0.0 {
            return;
        }

        let mut gain = self.gain * self.distance_gain(model, listener);
        gain = gain.max(self.min_gain).min(self.max_gain) * listener.gain;
        if !gain.is_finite() || gain < 0.0 {
            gain = 0.0;
        }
        let (pan_l, pan_r) = self.pan_gains(listener);
        let doppler = self.doppler_pitch(listener, doppler_factor, speed_of_sound);

        for i in 0..frames {
            // Advance past exhausted/missing buffers. Bound to one pass over
            // the queue: a looping source with no playable buffer would
            // otherwise spin forever here, holding the mixer lock.
            let mut scanned = 0usize;
            let current = loop {
                if scanned > self.queue.len() {
                    break None;
                }
                scanned += 1;
                let Some(&bid) = self.queue.get(self.current) else {
                    self.state = SourceState::Stopped;
                    break None;
                };
                let Some(buf) = buffers.get(&bid) else {
                    self.advance_buffer();
                    if self.state != SourceState::Playing {
                        break None;
                    }
                    continue;
                };
                let fc = buf.frame_count();
                if fc == 0 {
                    self.advance_buffer();
                    if self.state != SourceState::Playing {
                        break None;
                    }
                    continue;
                }
                if (self.sample_pos as usize) < fc {
                    break Some((buf, fc));
                }
                self.sample_pos -= fc as f64;
                self.advance_buffer();
                if self.state != SourceState::Playing {
                    break None;
                }
            };
            let Some((buf, fc)) = current else {
                break;
            };

            let pos = self.sample_pos.max(0.0);
            let idx = pos.floor() as usize;
            let frac = (pos - idx as f64) as f32;
            let i0 = idx.min(fc - 1);
            let i1 = (idx + 1).min(fc - 1);
            let bc = buf.channels as usize;
            let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;

            let (l, r) = if bc >= 2 {
                let l = lerp(buf.samples[i0 * bc], buf.samples[i1 * bc], frac);
                let r = lerp(buf.samples[i0 * bc + 1], buf.samples[i1 * bc + 1], frac);
                (l, r)
            } else {
                let s = lerp(buf.samples[i0 * bc], buf.samples[i1 * bc], frac);
                (s * pan_l, s * pan_r)
            };

            if channels >= 2 {
                out[i * channels] += l * gain;
                out[i * channels + 1] += r * gain;
            } else if channels == 1 {
                out[i * channels] += 0.5 * (l + r) * gain;
            }

            let step =
                (buf.sample_rate as f64 / device_rate) * (self.pitch as f64) * (doppler as f64);
            self.sample_pos += if step.is_finite() && step > 0.0 {
                step
            } else {
                0.0
            };
        }
    }
}

#[derive(Clone, Copy)]
struct Listener {
    gain: f32,
    position: [f32; 3],
    velocity: [f32; 3],
    orientation: [f32; 6],
}
impl Default for Listener {
    fn default() -> Self {
        Listener {
            gain: 1.0,
            position: [0.0; 3],
            velocity: [0.0; 3],
            // "at" facing -Z, "up" +Y.
            orientation: [0.0, 0.0, -1.0, 0.0, 1.0, 0.0],
        }
    }
}

struct ContextState {
    sources: HashMap<ALuint, Source>,
    buffers: HashMap<ALuint, Buffer>,
    next_source_id: ALuint,
    next_buffer_id: ALuint,
    listener: Listener,
    distance_model: ALenum,
    doppler_factor: f32,
    /// Stored for `alDopplerVelocity` (deprecated in OpenAL 1.1); the mixer
    /// uses `speed_of_sound` for Doppler instead.
    #[allow(dead_code)]
    doppler_velocity: f32,
    speed_of_sound: f32,
    error: ALenum,
}
impl Default for ContextState {
    fn default() -> Self {
        ContextState {
            sources: HashMap::new(),
            buffers: HashMap::new(),
            next_source_id: 1,
            next_buffer_id: 1,
            listener: Listener::default(),
            distance_model: AL_INVERSE_DISTANCE_CLAMPED,
            doppler_factor: 1.0,
            doppler_velocity: 1.0,
            speed_of_sound: 343.3,
            error: AL_NO_ERROR,
        }
    }
}

#[derive(Default)]
struct MixerState {
    device_rate: f64,
    out_channels: usize,
    contexts: HashMap<u32, ContextState>,
    next_context_id: u32,
}
impl MixerState {
    fn new() -> Self {
        MixerState {
            device_rate: 44100.0,
            out_channels: 2,
            contexts: HashMap::new(),
            next_context_id: 1,
        }
    }

    /// Mix all playing sources of all contexts into the interleaved output.
    fn mix(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() {
            *s = 0.0;
        }
        let channels = self.out_channels.max(1);
        let frames = out.len() / channels;
        let device_rate = self.device_rate;

        for ctx in self.contexts.values_mut() {
            let ContextState {
                ref buffers,
                ref mut sources,
                listener,
                distance_model,
                doppler_factor,
                speed_of_sound,
                ..
            } = *ctx;
            for src in sources.values_mut() {
                src.render(
                    out,
                    frames,
                    channels,
                    buffers,
                    device_rate,
                    distance_model,
                    doppler_factor,
                    speed_of_sound,
                    &listener,
                );
            }
        }

        // Guard against clipping from summed sources.
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

type SharedMixer = Arc<Mutex<MixerState>>;

/// The SDL audio callback: pulls samples from the shared mixer state.
pub struct MixerCallback {
    mixer: SharedMixer,
}
impl AudioCallback for MixerCallback {
    type Channel = f32;
    fn callback(&mut self, out: &mut [f32]) {
        let mut state = self.mixer.lock().unwrap();
        state.mix(out);
    }
}

/// Wrapper used only to move rust-sdl2's `!Send` audio handles across a scoped
/// worker thread when opening the device off the main thread (see
/// [OpenALManager::new] for the safety rationale).
struct AssertSend<T>(T);
// SAFETY: the !Send comes only from SdlDrop's thread-pinning marker, checked
// solely in Sdl::new. We never call SDL_Init/Quit on the worker, only
// SDL_OpenAudioDevice, which is thread-safe.
unsafe impl<T> Send for AssertSend<T> {}

// === Public API ===

static OPENALMANAGER_INSTANCE_EXISTS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Owns the SDL audio device and the shared mixer state. There may only be one
/// at a time.
pub struct OpenALManager {
    mixer: SharedMixer,
    // These keep the SDL audio device and subsystem alive; dropping the device
    // stops audio output. `None` if no audio device could be opened (e.g.
    // headless with no audio driver).
    _device: Option<AudioDevice<MixerCallback>>,
    _audio_subsystem: Option<AudioSubsystem>,
}
impl OpenALManager {
    pub fn new(audio_subsystem: Option<AudioSubsystem>) -> Result<Self, String> {
        if OPENALMANAGER_INSTANCE_EXISTS.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Err("Only one OpenALManager can exist at a time!".to_string());
        }

        let mixer: SharedMixer = Arc::new(Mutex::new(MixerState::new()));

        let device = if let Some(ref audio_subsystem) = audio_subsystem {
            let desired = AudioSpecDesired {
                freq: Some(44100),
                channels: Some(2),
                samples: Some(1024),
            };
            let mixer_for_cb = mixer.clone();
            let mixer_for_spec = mixer.clone();

            // Open on a worker thread: SDL_OpenAudioDevice activates the
            // AVAudioSession synchronously on iOS, which warns if done on
            // the main thread. Audio types are !Send, hence AssertSend.
            let subsystem_ref = AssertSend(audio_subsystem);
            let opened = std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        // Capture the whole Send wrapper, not just the inner
                        // !Send field (2021 disjoint capture would do that).
                        let subsystem_ref = subsystem_ref;
                        let audio_subsystem = subsystem_ref.0;
                        let device_name: Option<&str> = None;
                        audio_subsystem
                            .open_playback(device_name, &desired, move |spec: AudioSpec| {
                                {
                                    let mut state = mixer_for_spec.lock().unwrap();
                                    state.device_rate = spec.freq as f64;
                                    state.out_channels = spec.channels as usize;
                                }
                                MixerCallback {
                                    mixer: mixer_for_cb,
                                }
                            })
                            .map(|device| {
                                device.resume();
                                AssertSend(device)
                            })
                    })
                    .join()
                    .expect("audio device open thread panicked")
            });
            match opened {
                Ok(AssertSend(device)) => Some(device),
                Err(e) => {
                    log!("Warning: could not open SDL audio device, audio will be silent: {e}");
                    None
                }
            }
        } else {
            log!("Warning: no SDL audio subsystem available, audio will be silent");
            None
        };

        Ok(OpenALManager {
            mixer,
            _device: device,
            _audio_subsystem: audio_subsystem,
        })
    }
}
impl Drop for OpenALManager {
    fn drop(&mut self) {
        OPENALMANAGER_INSTANCE_EXISTS.store(false, std::sync::atomic::Ordering::SeqCst);

        // Close on a worker thread too: SDL_CloseAudioDevice hits the same
        // main-thread AVAudioSession warning as opening does.
        if let Some(device) = self._device.take() {
            let device = AssertSend(device);
            std::thread::spawn(move || {
                let device = device; // force whole-struct capture (Send)
                drop(device.0); // SDL_CloseAudioDevice runs here, off the main thread
            })
            .join()
            .expect("audio device close thread panicked");
        }
    }
}

/// A rendering context. Sources and buffers belong to the context that created
/// them.
pub struct OpenALContext {
    id: u32,
    mixer: SharedMixer,
    /// The (opaque) device token this context was created with.
    device: *mut alc_types::ALCdevice,
}
impl std::fmt::Debug for OpenALContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenALContext")
            .field("id", &self.id)
            .field("device", &self.device)
            .finish()
    }
}
impl OpenALContext {
    pub fn new(manager: &mut OpenALManager) -> Result<Self, String> {
        let device = unsafe { alcOpenDevice(std::ptr::null()) };
        if device.is_null() {
            return Err("Could not open audio device".to_string());
        }
        unsafe { Self::new_with_device_and_attrlist(manager, device, std::ptr::null()) }
    }

    /// # Safety
    /// `attrlist`, if non-null, must point to a null-terminated OpenAL
    /// attribute list.
    pub unsafe fn new_with_device_and_attrlist(
        manager: &mut OpenALManager,
        device: *mut alc_types::ALCdevice,
        _attrlist: *const ALCint,
    ) -> Result<Self, String> {
        // Attributes such as ALC_FREQUENCY are accepted but ignored: the mixer
        // always renders at the SDL device's rate.
        let id = {
            let mut state = manager.mixer.lock().unwrap();
            let id = state.next_context_id;
            state.next_context_id += 1;
            state.contexts.insert(id, ContextState::default());
            id
        };
        log_dbg!("New audio context {} on device {:?}", id, device);
        Ok(OpenALContext {
            id,
            mixer: manager.mixer.clone(),
            device,
        })
    }

    pub fn make_current<'al_ctx, 'manager: 'al_ctx>(
        &'al_ctx mut self,
        _manager: &'manager mut OpenALManager,
    ) -> OpenAL<'al_ctx> {
        OpenAL {
            mixer: self.mixer.clone(),
            ctx_id: self.id,
            _al_lifetime: PhantomData,
        }
    }

    pub fn SuspendContext(&mut self) {
        // No-op: this mixer processes changes immediately.
    }

    pub fn ProcessContext(&mut self) {
        // No-op: this mixer processes changes immediately.
    }

    pub fn GetContextsDevice(&self) -> *mut alc_types::ALCdevice {
        self.device
    }

    /// # Safety
    /// `enumName` must be a valid null-terminated C string.
    pub unsafe fn GetEnumValue(enumName: *const ALchar) -> ALenum {
        if enumName.is_null() {
            return AL_NONE;
        }
        let name = ffi::CStr::from_ptr(enumName);
        enum_value_from_name(name.to_bytes())
    }
}
impl Drop for OpenALContext {
    fn drop(&mut self) {
        if let Ok(mut state) = self.mixer.lock() {
            state.contexts.remove(&self.id);
        }
    }
}

/// A handle to a "current" context, mirroring the borrow-checked lifetime of
/// the old OpenAL wrapper. Each method briefly locks the shared mixer state.
pub struct OpenAL<'al_ctx> {
    mixer: SharedMixer,
    ctx_id: u32,
    _al_lifetime: PhantomData<&'al_ctx ()>,
}

impl OpenAL<'_> {
    fn with_ctx<R>(&self, default: R, f: impl FnOnce(&mut ContextState) -> R) -> R {
        let mut state = self.mixer.lock().unwrap();
        match state.contexts.get_mut(&self.ctx_id) {
            Some(ctx) => f(ctx),
            None => default,
        }
    }

    fn with_source<R>(&self, source: ALuint, default: R, f: impl FnOnce(&mut Source) -> R) -> R {
        let mut state = self.mixer.lock().unwrap();
        match state
            .contexts
            .get_mut(&self.ctx_id)
            .and_then(|ctx| ctx.sources.get_mut(&source))
        {
            Some(src) => f(src),
            None => default,
        }
    }

    pub unsafe fn GetError(&self) -> ALenum {
        self.with_ctx(AL_NO_ERROR, |ctx| {
            std::mem::replace(&mut ctx.error, AL_NO_ERROR)
        })
    }

    pub unsafe fn DistanceModel(&self, value: ALenum) {
        self.with_ctx((), |ctx| ctx.distance_model = value);
    }

    pub unsafe fn IsBuffer(&self, buffer: ALuint) -> ALboolean {
        self.with_ctx(AL_FALSE, |ctx| {
            if ctx.buffers.contains_key(&buffer) {
                AL_TRUE
            } else {
                AL_FALSE
            }
        })
    }
    pub unsafe fn IsSource(&self, source: ALuint) -> ALboolean {
        self.with_ctx(AL_FALSE, |ctx| {
            if ctx.sources.contains_key(&source) {
                AL_TRUE
            } else {
                AL_FALSE
            }
        })
    }
    /// # Safety
    /// `extName` must be a valid null-terminated C string.
    pub unsafe fn IsExtensionPresent(&self, extName: *const ALchar) -> ALboolean {
        if extName.is_null() {
            return AL_FALSE;
        }
        let name = ffi::CStr::from_ptr(extName).to_bytes();
        match name {
            b"AL_EXT_STATIC_BUFFER"
            | b"AL_EXT_OFFSET"
            | b"AL_EXT_LINEAR_DISTANCE"
            | b"AL_EXT_EXPONENT_DISTANCE" => AL_TRUE,
            _ => AL_FALSE,
        }
    }

    pub unsafe fn Enable(&self, _capability: ALenum) {
        // No capabilities are meaningful for this mixer.
    }

    /// # Safety
    /// `value` must point to writable storage for one `ALint`.
    pub unsafe fn GetBufferi(&self, buffer: ALuint, param: ALenum, value: *const ALint) {
        let value = value as *mut ALint;
        if value.is_null() {
            return;
        }
        let result = self.with_ctx(0, |ctx| {
            let Some(buf) = ctx.buffers.get(&buffer) else {
                ctx.error = AL_INVALID_NAME;
                return 0;
            };
            match param {
                AL_FREQUENCY => buf.sample_rate as ALint,
                AL_BITS => buf.bits as ALint,
                AL_CHANNELS => buf.channels as ALint,
                AL_SIZE => (buf.samples.len() * (buf.bits as usize / 8)) as ALint,
                _ => 0,
            }
        });
        *value = result;
    }

    pub unsafe fn Listenerf(&self, param: ALenum, value: ALfloat) {
        self.with_ctx((), |ctx| {
            if param == AL_GAIN {
                ctx.listener.gain = value;
            }
        });
    }
    pub unsafe fn Listener3f(
        &self,
        param: ALenum,
        value1: ALfloat,
        value2: ALfloat,
        value3: ALfloat,
    ) {
        self.with_ctx((), |ctx| match param {
            AL_POSITION => ctx.listener.position = [value1, value2, value3],
            AL_VELOCITY => ctx.listener.velocity = [value1, value2, value3],
            _ => {}
        });
    }
    /// # Safety
    /// `values` must point to enough `ALfloat`s for `param`.
    pub unsafe fn Listenerfv(&self, param: ALenum, values: *const ALfloat) {
        if values.is_null() {
            return;
        }
        self.with_ctx((), |ctx| match param {
            AL_GAIN => ctx.listener.gain = *values,
            AL_POSITION => ctx.listener.position = read3(values),
            AL_VELOCITY => ctx.listener.velocity = read3(values),
            AL_ORIENTATION => {
                for (i, o) in ctx.listener.orientation.iter_mut().enumerate() {
                    *o = *values.add(i);
                }
            }
            _ => {}
        });
    }
    pub unsafe fn Listeneri(&self, _param: ALenum, _value: ALint) {}
    pub unsafe fn Listener3i(&self, param: ALenum, value1: ALint, value2: ALint, value3: ALint) {
        self.Listener3f(param, value1 as f32, value2 as f32, value3 as f32)
    }
    /// # Safety
    /// `values` must point to enough `ALint`s for `param`.
    pub unsafe fn Listeneriv(&self, _param: ALenum, _values: *const ALint) {}

    /// # Safety
    /// `value` must point to writable storage for one `ALfloat`.
    pub unsafe fn GetListenerf(&self, param: ALenum, value: *mut ALfloat) {
        if value.is_null() {
            return;
        }
        let v = self.with_ctx(0.0, |ctx| {
            if param == AL_GAIN {
                ctx.listener.gain
            } else {
                0.0
            }
        });
        *value = v;
    }
    /// # Safety
    /// The `value*` pointers must point to writable `ALfloat` storage.
    pub unsafe fn GetListener3f(
        &self,
        param: ALenum,
        value1: *mut ALfloat,
        value2: *mut ALfloat,
        value3: *mut ALfloat,
    ) {
        let v = self.with_ctx([0.0; 3], |ctx| match param {
            AL_POSITION => ctx.listener.position,
            AL_VELOCITY => ctx.listener.velocity,
            _ => [0.0; 3],
        });
        if !value1.is_null() {
            *value1 = v[0];
        }
        if !value2.is_null() {
            *value2 = v[1];
        }
        if !value3.is_null() {
            *value3 = v[2];
        }
    }
    /// # Safety
    /// `values` must point to enough writable `ALfloat`s for `param`.
    pub unsafe fn GetListenerfv(&self, param: ALenum, values: *mut ALfloat) {
        if values.is_null() {
            return;
        }
        self.with_ctx((), |ctx| match param {
            AL_GAIN => *values = ctx.listener.gain,
            AL_POSITION => write3(values, ctx.listener.position),
            AL_VELOCITY => write3(values, ctx.listener.velocity),
            AL_ORIENTATION => {
                for (i, o) in ctx.listener.orientation.iter().enumerate() {
                    *values.add(i) = *o;
                }
            }
            _ => {}
        });
    }
    /// # Safety
    /// `value` must point to writable storage for one `ALint`.
    pub unsafe fn GetListeneri(&self, _param: ALenum, value: *mut ALint) {
        if !value.is_null() {
            *value = 0;
        }
    }
    /// # Safety
    /// The `value*` pointers must point to writable `ALint` storage.
    pub unsafe fn GetListener3i(
        &self,
        param: ALenum,
        value1: *mut ALint,
        value2: *mut ALint,
        value3: *mut ALint,
    ) {
        let v = self.with_ctx([0.0f32; 3], |ctx| match param {
            AL_POSITION => ctx.listener.position,
            AL_VELOCITY => ctx.listener.velocity,
            _ => [0.0; 3],
        });
        if !value1.is_null() {
            *value1 = v[0] as ALint;
        }
        if !value2.is_null() {
            *value2 = v[1] as ALint;
        }
        if !value3.is_null() {
            *value3 = v[2] as ALint;
        }
    }
    /// # Safety
    /// `values` must point to enough writable `ALint`s for `param`.
    pub unsafe fn GetListeneriv(&self, _param: ALenum, _values: *mut ALint) {}

    /// # Safety
    /// `sources` must point to writable storage for `n` `ALuint`s.
    pub unsafe fn GenSources(&self, n: ALsizei, sources: *mut ALuint) {
        if sources.is_null() || n <= 0 {
            return;
        }
        self.with_ctx((), |ctx| {
            for i in 0..n as usize {
                let id = ctx.next_source_id;
                ctx.next_source_id += 1;
                ctx.sources.insert(id, Source::default());
                *sources.add(i) = id;
            }
        });
    }
    /// # Safety
    /// `sources` must point to `n` readable `ALuint`s.
    pub unsafe fn DeleteSources(&self, n: ALsizei, sources: *const ALuint) {
        if sources.is_null() || n <= 0 {
            return;
        }
        self.with_ctx((), |ctx| {
            for i in 0..n as usize {
                let id = *sources.add(i);
                ctx.sources.remove(&id);
            }
        });
    }

    pub unsafe fn Sourcef(&self, source: ALuint, param: ALenum, value: ALfloat) {
        self.with_source(source, (), |src| match param {
            AL_GAIN => src.gain = value,
            AL_MIN_GAIN => src.min_gain = value,
            AL_MAX_GAIN => src.max_gain = value,
            AL_PITCH => src.pitch = value.max(0.0),
            AL_REFERENCE_DISTANCE => src.reference_distance = value,
            AL_MAX_DISTANCE => src.max_distance = value,
            AL_ROLLOFF_FACTOR => src.rolloff_factor = value,
            _ => {}
        });
    }
    pub unsafe fn Source3f(
        &self,
        source: ALuint,
        param: ALenum,
        value1: ALfloat,
        value2: ALfloat,
        value3: ALfloat,
    ) {
        self.with_source(source, (), |src| match param {
            AL_POSITION => src.position = [value1, value2, value3],
            AL_VELOCITY => src.velocity = [value1, value2, value3],
            AL_DIRECTION => src.direction = [value1, value2, value3],
            _ => {}
        });
    }
    /// # Safety
    /// `values` must point to enough `ALfloat`s for `param`.
    pub unsafe fn Sourcefv(&self, source: ALuint, param: ALenum, values: *const ALfloat) {
        if values.is_null() {
            return;
        }
        match param {
            AL_POSITION | AL_VELOCITY | AL_DIRECTION => {
                let v = read3(values);
                self.Source3f(source, param, v[0], v[1], v[2]);
            }
            _ => {
                let v = *values;
                self.Sourcef(source, param, v);
            }
        }
    }
    pub unsafe fn Sourcei(&self, source: ALuint, param: ALenum, value: ALint) {
        self.with_source(source, (), |src| match param {
            AL_BUFFER => {
                let buffer = value as ALuint;
                if buffer == 0 {
                    src.static_buffer = 0;
                    src.is_static = false;
                    src.queue.clear();
                } else {
                    src.static_buffer = buffer;
                    src.is_static = true;
                    src.queue.clear();
                    src.queue.push_back(buffer);
                }
                src.current = 0;
                src.sample_pos = 0.0;
                src.processed = 0;
            }
            AL_LOOPING => src.looping = value != 0,
            AL_SOURCE_RELATIVE => src.source_relative = value != 0,
            _ => {}
        });
    }
    pub unsafe fn Source3i(
        &self,
        source: ALuint,
        param: ALenum,
        value1: ALint,
        value2: ALint,
        value3: ALint,
    ) {
        self.Source3f(source, param, value1 as f32, value2 as f32, value3 as f32)
    }
    /// # Safety
    /// `values` must point to enough `ALint`s for `param`.
    pub unsafe fn Sourceiv(&self, source: ALuint, param: ALenum, values: *const ALint) {
        if !values.is_null() {
            self.Sourcei(source, param, *values);
        }
    }

    /// # Safety
    /// `value` must point to writable storage for one `ALfloat`.
    pub unsafe fn GetSourcef(&self, source: ALuint, param: ALenum, value: *mut ALfloat) {
        if value.is_null() {
            return;
        }
        let v = self.with_source(source, 0.0, |src| match param {
            AL_GAIN => src.gain,
            AL_MIN_GAIN => src.min_gain,
            AL_MAX_GAIN => src.max_gain,
            AL_PITCH => src.pitch,
            AL_REFERENCE_DISTANCE => src.reference_distance,
            AL_MAX_DISTANCE => src.max_distance,
            AL_ROLLOFF_FACTOR => src.rolloff_factor,
            _ => 0.0,
        });
        *value = v;
    }
    /// # Safety
    /// The `value*` pointers must point to writable `ALfloat` storage.
    pub unsafe fn GetSource3f(
        &self,
        source: ALuint,
        param: ALenum,
        value1: *mut ALfloat,
        value2: *mut ALfloat,
        value3: *mut ALfloat,
    ) {
        let v = self.with_source(source, [0.0; 3], |src| match param {
            AL_POSITION => src.position,
            AL_VELOCITY => src.velocity,
            AL_DIRECTION => src.direction,
            _ => [0.0; 3],
        });
        if !value1.is_null() {
            *value1 = v[0];
        }
        if !value2.is_null() {
            *value2 = v[1];
        }
        if !value3.is_null() {
            *value3 = v[2];
        }
    }
    /// # Safety
    /// `values` must point to enough writable `ALfloat`s for `param`.
    pub unsafe fn GetSourcefv(&self, source: ALuint, param: ALenum, values: *mut ALfloat) {
        if values.is_null() {
            return;
        }
        match param {
            AL_POSITION | AL_VELOCITY | AL_DIRECTION => {
                let v = self.with_source(source, [0.0; 3], |src| match param {
                    AL_POSITION => src.position,
                    AL_VELOCITY => src.velocity,
                    AL_DIRECTION => src.direction,
                    _ => [0.0; 3],
                });
                write3(values, v);
            }
            _ => self.GetSourcef(source, param, values),
        }
    }
    /// # Safety
    /// `value` must point to writable storage for one `ALint`.
    pub unsafe fn GetSourcei(&self, source: ALuint, param: ALenum, value: *mut ALint) {
        if value.is_null() {
            return;
        }
        if param == AL_SAMPLE_OFFSET {
            // The position in frames from the start of the queue, including
            // buffers that have been processed but not unqueued.
            *value = self.with_ctx(0, |ctx| {
                let Some(src) = ctx.sources.get(&source) else {
                    return 0;
                };
                let frames = |name: &ALuint| {
                    ctx.buffers
                        .get(name)
                        .map_or(0, |buf| buf.samples.len() / buf.channels.max(1) as usize)
                };
                let played: usize = src.queue.iter().take(src.current).map(frames).sum();
                let within = if src.current < src.queue.len() {
                    src.sample_pos as usize
                } else {
                    0
                };
                (played + within).try_into().unwrap_or(ALint::MAX)
            });
            return;
        }
        let v = self.with_source(source, 0, |src| match param {
            AL_SOURCE_STATE => src.al_state(),
            AL_BUFFERS_QUEUED => src.queue.len() as ALint,
            AL_BUFFERS_PROCESSED => src.processed as ALint,
            AL_BUFFER => src.static_buffer as ALint,
            AL_LOOPING => {
                if src.looping {
                    AL_TRUE as ALint
                } else {
                    AL_FALSE as ALint
                }
            }
            AL_SOURCE_RELATIVE => {
                if src.source_relative {
                    AL_TRUE as ALint
                } else {
                    AL_FALSE as ALint
                }
            }
            AL_SOURCE_TYPE => {
                if src.is_static {
                    AL_STATIC
                } else if src.queue.is_empty() {
                    AL_UNDETERMINED
                } else {
                    AL_STREAMING
                }
            }
            _ => 0,
        });
        *value = v;
    }
    /// # Safety
    /// The `value*` pointers must point to writable `ALint` storage.
    pub unsafe fn GetSource3i(
        &self,
        source: ALuint,
        param: ALenum,
        value1: *mut ALint,
        value2: *mut ALint,
        value3: *mut ALint,
    ) {
        let v = self.with_source(source, [0.0f32; 3], |src| match param {
            AL_POSITION => src.position,
            AL_VELOCITY => src.velocity,
            AL_DIRECTION => src.direction,
            _ => [0.0; 3],
        });
        if !value1.is_null() {
            *value1 = v[0] as ALint;
        }
        if !value2.is_null() {
            *value2 = v[1] as ALint;
        }
        if !value3.is_null() {
            *value3 = v[2] as ALint;
        }
    }
    /// # Safety
    /// `values` must point to enough writable `ALint`s for `param`.
    pub unsafe fn GetSourceiv(&self, source: ALuint, param: ALenum, values: *mut ALint) {
        if !values.is_null() {
            self.GetSourcei(source, param, values);
        }
    }

    pub unsafe fn SourcePlay(&self, source: ALuint) {
        self.with_source(source, (), |src| src.play());
    }
    pub unsafe fn SourcePause(&self, source: ALuint) {
        self.with_source(source, (), |src| src.pause());
    }
    pub unsafe fn SourceStop(&self, source: ALuint) {
        self.with_source(source, (), |src| src.stop());
    }
    pub unsafe fn SourceRewind(&self, source: ALuint) {
        self.with_source(source, (), |src| src.rewind());
    }

    /// # Safety
    /// `sources` must point to `n` readable `ALuint`s.
    pub unsafe fn SourcePlayv(&self, n: ALsizei, sources: *const ALuint) {
        self.for_each_source(n, sources, |src| src.play());
    }
    /// # Safety
    /// `sources` must point to `n` readable `ALuint`s.
    pub unsafe fn SourcePausev(&self, n: ALsizei, sources: *const ALuint) {
        self.for_each_source(n, sources, |src| src.pause());
    }
    /// # Safety
    /// `sources` must point to `n` readable `ALuint`s.
    pub unsafe fn SourceStopv(&self, n: ALsizei, sources: *const ALuint) {
        self.for_each_source(n, sources, |src| src.stop());
    }
    /// # Safety
    /// `sources` must point to `n` readable `ALuint`s.
    pub unsafe fn SourceRewindv(&self, n: ALsizei, sources: *const ALuint) {
        self.for_each_source(n, sources, |src| src.rewind());
    }

    unsafe fn for_each_source(&self, n: ALsizei, sources: *const ALuint, f: impl Fn(&mut Source)) {
        if sources.is_null() || n <= 0 {
            return;
        }
        self.with_ctx((), |ctx| {
            for i in 0..n as usize {
                let id = *sources.add(i);
                if let Some(src) = ctx.sources.get_mut(&id) {
                    f(src);
                }
            }
        });
    }

    /// # Safety
    /// `buffers` must point to `nb` readable `ALuint`s.
    pub unsafe fn SourceQueueBuffers(&self, source: ALuint, nb: ALsizei, buffers: *const ALuint) {
        if buffers.is_null() || nb <= 0 {
            return;
        }
        self.with_source(source, (), |src| {
            src.is_static = false;
            src.static_buffer = 0;
            for i in 0..nb as usize {
                let id = *buffers.add(i);
                src.queue.push_back(id);
            }
            // Like OpenAL Soft, a stopped source has processed all its
            // buffers, including ones queued since it stopped.
            if src.state == SourceState::Stopped {
                src.processed = src.queue.len() as u32;
                src.current = src.queue.len();
            }
        });
    }
    /// # Safety
    /// `buffers` must point to writable storage for `nb` `ALuint`s.
    pub unsafe fn SourceUnqueueBuffers(&self, source: ALuint, nb: ALsizei, buffers: *mut ALuint) {
        if buffers.is_null() || nb <= 0 {
            return;
        }
        self.with_source(source, (), |src| {
            let count = (nb as usize)
                .min(src.processed as usize)
                .min(src.queue.len());
            for i in 0..count {
                let id = src.queue.pop_front().unwrap_or(0);
                *buffers.add(i) = id;
            }
            src.processed -= count as u32;
            src.current = src.current.saturating_sub(count);
        });
    }

    /// # Safety
    /// `buffers` must point to writable storage for `n` `ALuint`s.
    pub unsafe fn GenBuffers(&self, n: ALsizei, buffers: *mut ALuint) {
        if buffers.is_null() || n <= 0 {
            return;
        }
        self.with_ctx((), |ctx| {
            for i in 0..n as usize {
                let id = ctx.next_buffer_id;
                ctx.next_buffer_id += 1;
                ctx.buffers.insert(id, Buffer::empty());
                *buffers.add(i) = id;
            }
        });
    }
    /// # Safety
    /// `buffers` must point to `n` readable `ALuint`s.
    pub unsafe fn DeleteBuffers(&self, n: ALsizei, buffers: *const ALuint) {
        if buffers.is_null() || n <= 0 {
            return;
        }
        self.with_ctx((), |ctx| {
            for i in 0..n as usize {
                let id = *buffers.add(i);
                ctx.buffers.remove(&id);
            }
        });
    }

    /// # Safety
    /// `data` must point to `size` readable bytes in the given `format`.
    pub unsafe fn BufferData(
        &self,
        buffer: ALuint,
        format: ALenum,
        data: *const ALvoid,
        size: ALsizei,
        samplerate: ALsizei,
    ) {
        let (channels, bits) = match format {
            AL_FORMAT_MONO8 => (1u32, 8u32),
            AL_FORMAT_MONO16 => (1, 16),
            AL_FORMAT_STEREO8 => (2, 8),
            AL_FORMAT_STEREO16 => (2, 16),
            _ => {
                self.with_ctx((), |ctx| ctx.error = AL_INVALID_ENUM);
                return;
            }
        };
        let size = size.max(0) as usize;
        let bytes: &[u8] = if data.is_null() || size == 0 {
            &[]
        } else {
            std::slice::from_raw_parts(data as *const u8, size)
        };

        let samples: Vec<f32> = if bits == 8 {
            // 8-bit PCM is unsigned, with 128 as the zero level.
            bytes.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect()
        } else {
            bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| i16::from_le_bytes(*c) as f32 / 32768.0)
                .collect()
        };

        let new_buffer = Buffer {
            samples,
            channels,
            sample_rate: samplerate.max(1) as u32,
            bits,
        };
        self.with_ctx((), |ctx| {
            ctx.buffers.insert(buffer, new_buffer);
        });
    }

    pub unsafe fn DopplerFactor(&self, dopplerFactor: ALfloat) {
        self.with_ctx((), |ctx| ctx.doppler_factor = dopplerFactor.max(0.0));
    }
    pub unsafe fn DopplerVelocity(&self, dopplerVelocity: ALfloat) {
        self.with_ctx((), |ctx| ctx.doppler_velocity = dopplerVelocity);
    }
    pub unsafe fn SpeedOfSound(&self, speed: ALfloat) {
        self.with_ctx((), |ctx| ctx.speed_of_sound = speed);
    }
}

// === ALC free functions ===

/// # Safety
/// `_devicename`, if non-null, must be a valid null-terminated C string.
pub unsafe fn alcOpenDevice(_devicename: *const ALCchar) -> *mut alc_types::ALCdevice {
    // Devices are opaque tokens; the single real output is the SDL device owned
    // by the OpenALManager. Hand out a unique non-null pointer per open.
    Box::into_raw(Box::new(0u8)) as *mut alc_types::ALCdevice
}

/// # Safety
/// `device` must be a pointer previously returned by [alcOpenDevice] and not
/// yet closed.
pub unsafe fn alcCloseDevice(device: *mut alc_types::ALCdevice) -> ALCboolean {
    if device.is_null() {
        return ALC_FALSE;
    }
    drop(Box::from_raw(device as *mut u8));
    ALC_TRUE
}

/// # Safety
/// `_device` must be null or a valid device token.
pub unsafe fn alcGetError(_device: *mut alc_types::ALCdevice) -> ALCenum {
    ALC_NO_ERROR
}

/// # Safety
/// `_device` must be null or a valid device token.
pub unsafe fn alcGetString(_device: *mut alc_types::ALCdevice, param: ALCenum) -> *const ALCchar {
    let s: &[u8] = match param {
        ALC_DEVICE_SPECIFIER | ALC_DEFAULT_DEVICE_SPECIFIER => DEVICE_SPECIFIER,
        ALC_EXTENSIONS => b"\0",
        _ => b"\0",
    };
    s.as_ptr() as *const ALCchar
}

// === Helpers ===

unsafe fn read3(values: *const ALfloat) -> [f32; 3] {
    [*values, *values.add(1), *values.add(2)]
}
unsafe fn write3(values: *mut ALfloat, v: [f32; 3]) {
    *values = v[0];
    *values.add(1) = v[1];
    *values.add(2) = v[2];
}

fn enum_value_from_name(name: &[u8]) -> ALenum {
    match name {
        b"AL_NONE" => AL_NONE,
        b"AL_FALSE" => AL_FALSE as ALenum,
        b"AL_TRUE" => AL_TRUE as ALenum,
        b"AL_FORMAT_MONO8" => AL_FORMAT_MONO8,
        b"AL_FORMAT_MONO16" => AL_FORMAT_MONO16,
        b"AL_FORMAT_STEREO8" => AL_FORMAT_STEREO8,
        b"AL_FORMAT_STEREO16" => AL_FORMAT_STEREO16,
        b"AL_INVERSE_DISTANCE" => AL_INVERSE_DISTANCE,
        b"AL_INVERSE_DISTANCE_CLAMPED" => AL_INVERSE_DISTANCE_CLAMPED,
        b"AL_LINEAR_DISTANCE" => AL_LINEAR_DISTANCE,
        b"AL_LINEAR_DISTANCE_CLAMPED" => AL_LINEAR_DISTANCE_CLAMPED,
        b"AL_EXPONENT_DISTANCE" => AL_EXPONENT_DISTANCE,
        b"AL_EXPONENT_DISTANCE_CLAMPED" => AL_EXPONENT_DISTANCE_CLAMPED,
        _ => AL_NONE,
    }
}
