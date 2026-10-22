/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `CFArray` and `CFMutableArray`.
//!
//! These are toll-free bridged to `NSArray` and `NSMutableArray` in Apple's
//! implementation. Here they are the same types.

use super::cf_allocator::{kCFAllocatorDefault, CFAllocatorRef};
use super::CFIndex;
use crate::abi::GuestFunction;
use crate::dyld::{
    export_c_func, ConstantExports, Dyld, FunctionExports, HostConstant, HostFunction,
};
use crate::frameworks::core_foundation::{CFRelease, CFRetain};
use crate::frameworks::foundation::ns_array::ArrayHostObject;
use crate::frameworks::foundation::NSUInteger;
use crate::mem::{ConstVoidPtr, Mem, SafeRead};
use crate::objc::{id, msg, msg_class};
use crate::Environment;

#[allow(dead_code)]
pub type CFArrayRef = super::CFTypeRef;
pub type CFMutableArrayRef = super::CFTypeRef;

#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct CFArrayCallBacks {
    pub version: CFIndex,
    pub retain: GuestFunction,
    pub release: GuestFunction,
    pub copy_description: GuestFunction,
    pub equal: GuestFunction,
}
unsafe impl SafeRead for CFArrayCallBacks {}

fn default_retain(
    env: &mut Environment,
    _allocator: CFAllocatorRef,
    value: ConstVoidPtr,
) -> ConstVoidPtr {
    CFRetain(env, value.cast_mut().cast()).cast().cast_const()
}

fn default_release(env: &mut Environment, _allocator: CFAllocatorRef, value: ConstVoidPtr) {
    CFRelease(env, value.cast_mut().cast());
}

fn default_copy_description(env: &mut Environment, value: ConstVoidPtr) -> CFArrayRef {
    let object: id = value.cast_mut().cast();
    let description: id = msg![env; object description];
    msg![env; description copy]
}

fn default_equal(env: &mut Environment, left: ConstVoidPtr, right: ConstVoidPtr) -> bool {
    if left == right {
        return true;
    }
    let left: id = left.cast_mut().cast();
    let right: id = right.cast_mut().cast();
    msg![env; left isEqual:right]
}

fn make_default_callbacks(mem: &mut Mem, dyld: &mut Dyld) -> CFArrayCallBacks {
    let retain: HostFunction = &(default_retain as fn(&mut Environment, _, _) -> _);
    let release: HostFunction = &(default_release as fn(&mut Environment, _, _));
    let copy_description: HostFunction =
        &(default_copy_description as fn(&mut Environment, _) -> _);
    let equal: HostFunction = &(default_equal as fn(&mut Environment, _, _) -> _);
    CFArrayCallBacks {
        version: 0,
        retain: dyld.create_guest_function(mem, "__touchHLE_CFArray_retain", retain),
        release: dyld.create_guest_function(mem, "__touchHLE_CFArray_release", release),
        copy_description: dyld.create_guest_function(
            mem,
            "__touchHLE_CFArray_copyDescription",
            copy_description,
        ),
        equal: dyld.create_guest_function(mem, "__touchHLE_CFArray_equal", equal),
    }
}

pub const CONSTANTS: ConstantExports = &[(
    "_kCFTypeArrayCallBacks",
    HostConstant::Custom(|env| {
        let callbacks = make_default_callbacks(&mut env.mem, &mut env.dyld);
        let callbacks_ptr = env.mem.alloc_and_write(callbacks);
        env.mem.alloc_and_write(callbacks_ptr).cast().cast_const()
    }),
)];

fn CFArrayCreateMutable(
    env: &mut Environment,
    allocator: CFAllocatorRef,
    capacity: CFIndex,
    callbacks: ConstVoidPtr, // TODO, should be `const CFArrayCallBacks*`
) -> CFMutableArrayRef {
    // TODO: support custom allocators.
    assert!(allocator == kCFAllocatorDefault || env.mem.read(allocator).is_system_default());
    // CFArrayCreateMutable accepts zero for variable capacity and otherwise
    // a nonnegative requested capacity. Modern CoreFoundation grows as needed.
    assert!(capacity >= 0);
    let callbacks: Option<CFArrayCallBacks> = if callbacks.is_null() {
        None
    } else {
        let callbacks = env.mem.read(callbacks.cast::<CFArrayCallBacks>());
        let version = callbacks.version;
        assert_eq!(version, 0);
        Some(callbacks)
    };

    let array: id = msg_class![env; _touchHLE_NSMutableArray_non_retaining new];
    let host_object: &mut ArrayHostObject = env.objc.borrow_mut(array);
    host_object.array.reserve(capacity as usize);
    host_object.cf_callbacks = callbacks;
    array
}

fn CFArrayGetCount(env: &mut Environment, array: CFArrayRef) -> CFIndex {
    let count: NSUInteger = msg![env; array count];
    count.try_into().unwrap()
}

fn CFArrayGetValueAtIndex(env: &mut Environment, array: CFArrayRef, idx: CFIndex) -> ConstVoidPtr {
    let idx: NSUInteger = idx.try_into().unwrap();
    let value: id = msg![env; array objectAtIndex:idx];
    value.cast().cast_const()
}

fn CFArrayAppendValue(env: &mut Environment, array: CFMutableArrayRef, value: ConstVoidPtr) {
    let value: id = value.cast().cast_mut();
    msg![env; array addObject:value]
}

fn CFArrayRemoveValueAtIndex(env: &mut Environment, array: CFMutableArrayRef, idx: CFIndex) {
    let idx: NSUInteger = idx.try_into().unwrap();
    msg![env; array removeObjectAtIndex:idx]
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(CFArrayCreateMutable(_, _, _)),
    export_c_func!(CFArrayGetCount(_)),
    export_c_func!(CFArrayGetValueAtIndex(_, _)),
    export_c_func!(CFArrayAppendValue(_, _)),
    export_c_func!(CFArrayRemoveValueAtIndex(_, _)),
];
