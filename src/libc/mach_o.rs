/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `Mach-O` related functions.

use crate::dyld::{export_c_func, FunctionExports};
use crate::mem::{ConstPtr, GuestUSize, MutPtr, Ptr, SafeRead};
use std::collections::HashMap;
use crate::Environment;

/// The 32-bit guest layout of struct NXArchInfo in <mach-o/arch.h>.
/// Pointers refer to guest strings, not host process addresses.
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct NXArchInfo {
    name: ConstPtr<u8>,
    cputype: i32,
    cpusubtype: i32,
    byteorder: i32,
    description: ConstPtr<u8>,
}
unsafe impl SafeRead for NXArchInfo {}

#[derive(Default)]
pub struct State {
    arch_info_cache: HashMap<(i32, i32), ConstPtr<NXArchInfo>>,
}

/// Returns canonical name, description and subtype for known 32-bit ARM
/// architectures. Subtype capability flags occupy the high eight bits.
fn arm_architecture(cputype: i32, subtype: i32) -> Option<(&'static str, &'static str, i32)> {
    if cputype != 12 {
        return None;
    }
    let subtype = if subtype == -1 { 0 } else { subtype & 0x00ff_ffff };
    let (name, description) = match subtype {
        0 => ("arm", "ARM"),
        5 => ("armv4t", "ARM v4t"),
        6 => ("armv6", "ARM v6"),
        7 => ("armv5tej", "ARM v5tej"),
        9 => ("armv7", "ARM v7"),
        10 => ("armv7f", "ARM v7f"),
        11 => ("armv7s", "ARM v7s"),
        12 => ("armv7k", "ARM v7k"),
        _ => return None,
    };
    Some((name, description, subtype))
}

fn NXGetArchInfoFromCpuType(
    env: &mut Environment,
    cputype: i32,
    cpusubtype: i32,
) -> ConstPtr<NXArchInfo> {
    let Some((name, description, canonical_subtype)) = arm_architecture(cputype, cpusubtype) else {
        return Ptr::null();
    };
    let key = (cputype, canonical_subtype);
    if let Some(&info) = env.libc_state.mach_o.arch_info_cache.get(&key) {
        return info;
    }

    let name = env.mem.alloc_and_write_cstr(name.as_bytes()).cast_const();
    let description = env
        .mem
        .alloc_and_write_cstr(description.as_bytes())
        .cast_const();
    let info = NXArchInfo {
        name,
        cputype,
        cpusubtype: canonical_subtype,
        byteorder: 1, // NX_LittleEndian
        description,
    };
    let pointer = env.mem.alloc_and_write(info).cast_const();
    env.libc_state.mach_o.arch_info_cache.insert(key, pointer);
    pointer
}

fn _NSGetExecutablePath(env: &mut Environment, buf: MutPtr<u8>, buf_size: MutPtr<u32>) -> i32 {
    let binding = env.bundle.executable_path();
    let bin_path = binding.as_str();

    let size = env.mem.read(buf_size);
    let len: GuestUSize = bin_path.len().try_into().unwrap();
    assert!(len < size);

    env.mem
        .bytes_at_mut(buf, len)
        .copy_from_slice(bin_path.as_bytes());
    env.mem.write(buf + len, b'\0');
    0
}

fn get_end(env: &mut Environment) -> u32 {
    // Assume app binary is the first.
    // From https://www.manpagez.com/man/3/get_end/
    // `In a Mach-O file <...> get_end returns the first address after
    // the last segment in the executable`
    // It was confirmed on a real device with the TestApp binary.
    env.bins[0].last_segment_end
}

fn get_etext(env: &mut Environment) -> u32 {
    // Assume app binary is the first.
    let app_sections = &env.bins[0].sections;
    assert_eq!(
        app_sections
            .iter()
            .filter(|s| s.name.to_uppercase() == "__TEXT")
            .count(),
        1
    );
    let text_section = app_sections
        .iter()
        .find(|s| s.name.to_uppercase() == "__TEXT")
        .unwrap();
    text_section.next_section_addr()
}

// The main executable is the first guest Mach-O in env.bins, followed by
// any loaded guest dylibs. Host-implemented frameworks do not have Mach-O
// images and therefore are not included.
fn _dyld_image_count(env: &mut Environment) -> u32 {
    env.bins.len().try_into().unwrap()
}

// Each Mach-O's __TEXT segment starts at its mapped mach_header. This
// returns NULL for an out-of-range index, as dyld does on iOS.
fn _dyld_get_image_header(env: &mut Environment, image_index: u32) -> ConstPtr<u8> {
    let Some(bin) = env.bins.get(image_index as usize) else {
        return Ptr::null();
    };
    match bin.text_segment_base {
        Some(base) => Ptr::from_bits(base),
        None => Ptr::null(),
    }
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(NXGetArchInfoFromCpuType(_, _)),
    export_c_func!(_dyld_image_count()),
    export_c_func!(_dyld_get_image_header(_)),
    export_c_func!(_NSGetExecutablePath(_, _)),
    export_c_func!(get_end()),
    export_c_func!(get_etext()),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architecture_lookup_matches_32_bit_armv7_abi() {
        assert_eq!(std::mem::size_of::<NXArchInfo>(), 20);
        assert_eq!(arm_architecture(12, 9), Some(("armv7", "ARM v7", 9)));
        assert_eq!(arm_architecture(12, 0x0100_0009), Some(("armv7", "ARM v7", 9)));
        assert_eq!(arm_architecture(12, -1), Some(("arm", "ARM", 0)));
        assert!(arm_architecture(7, 9).is_none());
        assert!(arm_architecture(12, 31).is_none());
    }
}
