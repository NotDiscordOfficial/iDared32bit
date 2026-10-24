/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! SCNetworkReachability

use crate::abi::{CallFromHost, GuestFunction};
use crate::frameworks::core_foundation::cf_run_loop::{CFRunLoopMode, CFRunLoopRef};
use crate::frameworks::foundation::ns_run_loop;
use crate::dyld::{export_c_func, FunctionExports};
use crate::frameworks::core_foundation::cf_allocator::{kCFAllocatorDefault, CFAllocatorRef};
use crate::frameworks::core_foundation::CFTypeRef;
use crate::libc::sys::socket::sockaddr;
use crate::mem::{ConstPtr, MutPtr, MutVoidPtr, Ptr};
use crate::objc::{msg, objc_classes, Class, ClassExports, HostObject};
use crate::Environment;
use std::net::SocketAddrV4;

type SCNetworkReachabilityFlags = u32;
const kSCNetworkReachabilityFlagsReachable: SCNetworkReachabilityFlags = 1 << 1;
const kSCNetworkReachabilityFlagsIsDirect: SCNetworkReachabilityFlags = 1 << 17;

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

// SCNetworkReachabilityRef is not explicitly stated to be CFType-based type,
// but the result of "Create" or "Copy" functions here is expected to be
// released with CFRelease().
@implementation _touchHLE_SCNetworkReachability: NSObject
@end

};

struct SCNetworkReachabilityHostObject {
    address: Option<SocketAddrV4>,
    callback: Option<GuestFunction>,
    callback_info: MutVoidPtr,
}
impl HostObject for SCNetworkReachabilityHostObject {}

// See comment for `_touchHLE_SCNetworkReachability` class
type SCNetworkReachabilityRef = CFTypeRef;

fn SCNetworkReachabilityCreateWithName(
    env: &mut Environment,
    allocator: CFAllocatorRef,
    name: ConstPtr<u8>,
) -> SCNetworkReachabilityRef {
    assert!(allocator == kCFAllocatorDefault || env.mem.read(allocator).is_system_default()); // unimplemented
    if env
        .bundle
        .bundle_identifier()
        .starts_with("com.chillingo.cuttherope")
        && env.mem.cstr_at_utf8(name).unwrap() == "chillingo-crystal.appspot.com"
    {
        log!("Applying game-specific hack for Cut the Rope: SCNetworkReachabilityCreateWithName(\"chillingo-crystal.appspot.com\") returns NULL");
        return Ptr::null();
    }
    let isa = env
        .objc
        .get_known_class("_touchHLE_SCNetworkReachability", &mut env.mem);
    let res = env.objc.alloc_object(
        isa,
        Box::new(SCNetworkReachabilityHostObject {
            address: None,
            callback: None,
            callback_info: Ptr::null(),
        }), // TODO
        &mut env.mem,
    );
    log!(
        "TODO: SCNetworkReachabilityCreateWithName({:?}, {:?} {:?}) -> {:?}",
        allocator,
        name,
        env.mem.cstr_at_utf8(name),
        res
    );
    res
}

fn SCNetworkReachabilityCreateWithAddress(
    env: &mut Environment,
    allocator: CFAllocatorRef,
    address: ConstPtr<sockaddr>,
) -> SCNetworkReachabilityRef {
    assert!(allocator == kCFAllocatorDefault || env.mem.read(allocator).is_system_default()); // unimplemented
    let isa = env
        .objc
        .get_known_class("_touchHLE_SCNetworkReachability", &mut env.mem);
    let address_val = env.mem.read(address);
    let res = env.objc.alloc_object(
        isa,
        Box::new(SCNetworkReachabilityHostObject {
            address: Some(address_val.to_sockaddr_v4()),
            callback: None,
            callback_info: Ptr::null(),
        }),
        &mut env.mem,
    );
    log_dbg!(
        "SCNetworkReachabilityCreateWithAddress({:?}, {:?} ({})) -> {:?}",
        allocator,
        address,
        address_val.to_sockaddr_v4(),
        res
    );
    res
}

fn SCNetworkReachabilityGetFlags(
    env: &mut Environment,
    target: SCNetworkReachabilityRef,
    flags: MutPtr<SCNetworkReachabilityFlags>,
) -> bool {
    if !env.options.network_access {
        log_dbg!(
            "Network access is disabled, SCNetworkReachabilityGetFlags({:?}, {:?}) -> false",
            target,
            flags
        );
        return false;
    }

    let target_class: Class = msg![env; target class];
    assert_eq!(
        target_class,
        env.objc
            .get_known_class("_touchHLE_SCNetworkReachability", &mut env.mem)
    );
    let host_object = env.objc.borrow::<SCNetworkReachabilityHostObject>(target);
    if let Some(addr) = host_object.address {
        if addr.ip().is_link_local() {
            log_dbg!(
                "SCNetworkReachabilityGetFlags({:?}, {:?}) -> true",
                target,
                flags
            );
            // Those corresponds to local WiFi connection on a real iOS device
            // TODO: actually check for the connectivity
            // (but do we _really_ need it?)
            let out_flags =
                kSCNetworkReachabilityFlagsReachable | kSCNetworkReachabilityFlagsIsDirect;
            env.mem.write(flags, out_flags);
            return true;
        }
    }
    log!(
        "TODO: SCNetworkReachabilityGetFlags({:?}, {:?}) -> false",
        target,
        flags
    );
    false
}


fn reachability_flags(env: &mut Environment, target: SCNetworkReachabilityRef) -> SCNetworkReachabilityFlags {
    if !env.options.network_access {
        return 0;
    }
    let address = env.objc.borrow::<SCNetworkReachabilityHostObject>(target).address;
    if address.is_some_and(|address| address.ip().is_link_local()) {
        kSCNetworkReachabilityFlagsReachable | kSCNetworkReachabilityFlagsIsDirect
    } else {
        0
    }
}

/// The run loop uses this to deliver the initial state and subsequent changes.
/// With network access disabled, the correct advertised state is unreachable.
pub(crate) fn callback_flags(
    env: &mut Environment,
    target: SCNetworkReachabilityRef,
) -> Option<SCNetworkReachabilityFlags> {
    env.objc
        .borrow::<SCNetworkReachabilityHostObject>(target)
        .callback
        .map(|_| reachability_flags(env, target))
}

pub(crate) fn notify_callback(
    env: &mut Environment,
    target: SCNetworkReachabilityRef,
    flags: SCNetworkReachabilityFlags,
) {
    let (callback, info) = {
        let state = env.objc.borrow::<SCNetworkReachabilityHostObject>(target);
        (state.callback, state.callback_info)
    };
    if let Some(callback) = callback {
        let () = callback.call_from_host(env, (target, flags, info));
    }
}

fn SCNetworkReachabilityScheduleWithRunLoop(
    env: &mut Environment,
    target: SCNetworkReachabilityRef,
    run_loop: CFRunLoopRef,
    mode: CFRunLoopMode,
) -> bool {
    if target.is_null() || run_loop.is_null() || mode.is_null() {
        return false;
    }
    let class: Class = msg![env; target class];
    if class != env.objc.get_known_class("_touchHLE_SCNetworkReachability", &mut env.mem) {
        return false;
    }
    ns_run_loop::add_reachability_source(env, run_loop, target, mode);
    true
}

fn SCNetworkReachabilityUnscheduleFromRunLoop(
    env: &mut Environment,
    target: SCNetworkReachabilityRef,
    run_loop: CFRunLoopRef,
    mode: CFRunLoopMode,
) -> bool {
    if target.is_null() || run_loop.is_null() || mode.is_null() {
        return false;
    }
    ns_run_loop::remove_reachability_source(env, run_loop, target, mode)
}

fn SCNetworkReachabilitySetCallback(
    env: &mut Environment,
    target: SCNetworkReachabilityRef,
    callout: GuestFunction, // SCNetworkReachabilityCallBack
    context: MutVoidPtr,    // SCNetworkReachabilityContext *
) -> bool {
    if target.is_null() {
        return false;
    }
    let target_class: Class = msg![env; target class];
    if target_class != env.objc.get_known_class("_touchHLE_SCNetworkReachability", &mut env.mem) {
        return false;
    }

    // SCNetworkReachabilityContext contains five 32-bit slots on ARMv7:
    // version, info, retain, release and copyDescription.
    // Do not claim to own a context whose retain/release hooks we cannot honor.
    let info = if context.is_null() || callout.addr_with_thumb_bit() == 0 {
        Ptr::null()
    } else {
        let slots = context.cast::<u32>();
        if env.mem.read(slots) != 0
            || env.mem.read(slots + 2) != 0
            || env.mem.read(slots + 3) != 0
        {
            log!("SCNetworkReachabilitySetCallback: unsupported context version or ownership hooks");
            return false;
        }
        Ptr::from_bits(env.mem.read(slots + 1))
    };
    let state = env.objc.borrow_mut::<SCNetworkReachabilityHostObject>(target);
    state.callback = (callout.addr_with_thumb_bit() != 0).then_some(callout);
    state.callback_info = info;
    true
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(SCNetworkReachabilityCreateWithName(_, _)),
    export_c_func!(SCNetworkReachabilityCreateWithAddress(_, _)),
    export_c_func!(SCNetworkReachabilityGetFlags(_, _)),
    export_c_func!(SCNetworkReachabilitySetCallback(_, _, _)),
    export_c_func!(SCNetworkReachabilityScheduleWithRunLoop(_, _, _)),
    export_c_func!(SCNetworkReachabilityUnscheduleFromRunLoop(_, _, _)),
];
