/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `CFUUID`.

use super::cf_allocator::{kCFAllocatorDefault, CFAllocatorRef};
use super::cf_string::CFStringRef;
use super::CFTypeRef;
use crate::abi::GuestArg;
use crate::dyld::{export_c_func, FunctionExports};
use crate::frameworks::foundation::ns_string::from_rust_string;
use crate::objc::{objc_classes, ClassExports, HostObject};
use crate::Environment;
use uuid::Uuid;

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

// CFUUID doesn't have a corresponding NS type (at least, not up until iOS 6+
// and even that one is _not_ toll-free bridged, see NSUUID docs),
// but the callers of CFUUIDCreate() are expected to call CFRelease() on them.
@implementation _touchHLE_CFUUID: NSObject
@end

};

/// Note: Apple is using a pointer to an opaque struct instead
type CFUUIDRef = CFTypeRef;

struct CFUUIDHostObject {
    uuid: Uuid,
}
impl HostObject for CFUUIDHostObject {}

/// CFUUIDBytes is passed by value in four 32-bit ARM argument slots.
#[derive(Debug)]
struct CFUUIDBytes([u8; 16]);

impl GuestArg for CFUUIDBytes {
    const REG_COUNT: usize = 4;

    fn from_regs(regs: &[u32]) -> Self {
        let mut bytes = [0u8; 16];
        for (chunk, word) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(regs.iter()) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        Self(bytes)
    }

    fn to_regs(self, regs: &mut [u32]) {
        for (word, chunk) in regs.iter_mut().zip(self.0.as_chunks::<4>().0.iter()) {
            *word = u32::from_le_bytes(*chunk);
        }
    }
}

fn create_uuid(env: &mut Environment, uuid: Uuid) -> CFUUIDRef {
    let host_obj = Box::new(CFUUIDHostObject { uuid });
    let class = env.objc.get_known_class("_touchHLE_CFUUID", &mut env.mem);
    env.objc.alloc_object(class, host_obj, &mut env.mem)
}

fn CFUUIDCreate(env: &mut Environment, allocator: CFAllocatorRef) -> CFUUIDRef {
    assert!(allocator == kCFAllocatorDefault || env.mem.read(allocator).is_system_default()); // unimplemented
    create_uuid(env, Uuid::new_v4())
}

fn CFUUIDCreateFromUUIDBytes(
    env: &mut Environment,
    allocator: CFAllocatorRef,
    bytes: CFUUIDBytes,
) -> CFUUIDRef {
    assert!(allocator == kCFAllocatorDefault || env.mem.read(allocator).is_system_default()); // unimplemented
    create_uuid(env, Uuid::from_bytes(bytes.0))
}

fn CFUUIDCreateString(
    env: &mut Environment,
    allocator: CFAllocatorRef,
    uuid: CFUUIDRef,
) -> CFStringRef {
    assert!(allocator == kCFAllocatorDefault || env.mem.read(allocator).is_system_default()); // unimplemented

    let host_object = env.objc.borrow::<CFUUIDHostObject>(uuid);
    let uuid_str = host_object.uuid.hyphenated().to_string().to_uppercase();
    from_rust_string(env, uuid_str)
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(CFUUIDCreate(_)),
    export_c_func!(CFUUIDCreateFromUUIDBytes(_, _)),
    export_c_func!(CFUUIDCreateString(_, _)),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_bytes_arm_argument_round_trip() {
        let bytes = CFUUIDBytes([
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ]);
        let mut regs = [0u32; 4];
        CFUUIDBytes::to_regs(bytes, &mut regs);
        assert_eq!(regs, [0x33221100, 0x77665544, 0xbbaa9988, 0xffeeddcc]);
        assert_eq!(
            CFUUIDBytes::from_regs(&regs).0,
            [
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff,
            ]
        );
    }
}
