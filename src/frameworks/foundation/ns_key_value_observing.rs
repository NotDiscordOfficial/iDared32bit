/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Key-value observing (`NSKeyValueObserving`), the observer side of the
//! `NSObject` methods.
//!
//! Notifications are sent when a class calls `willChangeValueForKey:` and
//! `didChangeValueForKey:`, as NSUserDefaults does. Automatic notification
//! for other classes' setters isn't implemented, and is logged.
//!
//! Resources:
//! - Apple's [Key-Value Observing Programming Guide](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/KeyValueObserving/KeyValueObserving.html)

use super::{ns_string, NSUInteger};
use crate::dyld::{ConstantExports, HostConstant};
use crate::mem::MutVoidPtr;
use crate::objc::{autorelease, id, msg, msg_class, nil, SEL};
use crate::Environment;

type NSKeyValueObservingOptions = NSUInteger;
const NSKeyValueObservingOptionNew: NSKeyValueObservingOptions = 1;
const NSKeyValueObservingOptionOld: NSKeyValueObservingOptions = 2;
const NSKeyValueObservingOptionInitial: NSKeyValueObservingOptions = 4;
const NSKeyValueObservingOptionPrior: NSKeyValueObservingOptions = 8;

/// `NSKeyValueChangeSetting`, the only kind supported.
const NSKeyValueChangeSetting: NSUInteger = 1;

const NSKeyValueChangeKindKey: &str = "kind";
const NSKeyValueChangeNewKey: &str = "new";
const NSKeyValueChangeOldKey: &str = "old";
const NSKeyValueChangeIndexesKey: &str = "indexes";
const NSKeyValueChangeNotificationIsPriorKey: &str = "notificationIsPrior";

pub const CONSTANTS: ConstantExports = &[
    (
        "_NSKeyValueChangeKindKey",
        HostConstant::NSString(NSKeyValueChangeKindKey),
    ),
    (
        "_NSKeyValueChangeNewKey",
        HostConstant::NSString(NSKeyValueChangeNewKey),
    ),
    (
        "_NSKeyValueChangeOldKey",
        HostConstant::NSString(NSKeyValueChangeOldKey),
    ),
    (
        "_NSKeyValueChangeIndexesKey",
        HostConstant::NSString(NSKeyValueChangeIndexesKey),
    ),
    (
        "_NSKeyValueChangeNotificationIsPriorKey",
        HostConstant::NSString(NSKeyValueChangeNotificationIsPriorKey),
    ),
];

struct Registration {
    /// The observed object. Not retained, as in Foundation.
    object: id,
    /// Not retained, as in Foundation.
    observer: id,
    key_path: String,
    options: NSKeyValueObservingOptions,
    context: MutVoidPtr,
}

#[derive(Default)]
pub struct State {
    registrations: Vec<Registration>,
    /// Old values saved by `willChangeValueForKey:` for the matching
    /// `didChangeValueForKey:`. Retained.
    old_values: Vec<(id, String, id)>,
}
impl State {
    fn get(env: &mut Environment) -> &mut State {
        &mut env.framework_state.foundation.ns_key_value_observing
    }
}

/// `addObserver:forKeyPath:options:context:`
pub fn add_observer(
    env: &mut Environment,
    object: id,
    observer: id,
    key_path: id,
    options: NSKeyValueObservingOptions,
    context: MutVoidPtr,
) {
    let key_path = ns_string::to_rust_string(env, key_path).into_owned();
    let ns_user_defaults_class = env.objc.get_known_class("NSUserDefaults", &mut env.mem);
    let class = msg![env; object class];
    if !env.objc.class_is_subclass_of(class, ns_user_defaults_class) {
        log!(
            "TODO: KVO only notifies when a class calls willChangeValueForKey: \
            and didChangeValueForKey:, not on {}'s setters",
            env.objc.get_class_name(class)
        );
    }
    if key_path.contains('.') {
        log!(
            "TODO: KVO key paths with dots ({:?}) are compared whole",
            key_path
        );
    }
    log_dbg!(
        "KVO: {:?} observes {:?} of {:?}",
        observer,
        key_path,
        object
    );
    State::get(env).registrations.push(Registration {
        object,
        observer,
        key_path: key_path.clone(),
        options,
        context,
    });
    if options & NSKeyValueObservingOptionInitial != 0 {
        let change = change_dict(env, object, &key_path, options, nil, false);
        notify(env, observer, &key_path, object, change, context);
    }
}

/// `removeObserver:forKeyPath:` and `removeObserver:forKeyPath:context:`
pub fn remove_observer(
    env: &mut Environment,
    object: id,
    observer: id,
    key_path: id,
    context: Option<MutVoidPtr>,
) {
    let key_path = ns_string::to_rust_string(env, key_path).into_owned();
    let registrations = &mut State::get(env).registrations;
    // Like Apple's, remove only the most recent matching registration.
    if let Some(i) = registrations.iter().rposition(|r| {
        r.object == object
            && r.observer == observer
            && r.key_path == key_path
            && context.is_none_or(|c| c == r.context)
    }) {
        registrations.remove(i);
    } else {
        log!(
            "Warning: {:?} removed as an observer of {:?} for {:?} it wasn't observing",
            observer,
            object,
            key_path
        );
    }
}

/// Forget an object that's being deallocated, as observed object or observer.
pub fn forget_object(env: &mut Environment, object: id) {
    let state = State::get(env);
    if state.registrations.is_empty() {
        return;
    }
    state
        .registrations
        .retain(|r| r.object != object && r.observer != object);
}

fn matching(env: &mut Environment, object: id, key: &str) -> Vec<(id, NSUInteger, MutVoidPtr)> {
    State::get(env)
        .registrations
        .iter()
        .filter(|r| r.object == object && r.key_path == key)
        .map(|r| (r.observer, r.options, r.context))
        .collect()
}

/// `willChangeValueForKey:`
pub fn will_change(env: &mut Environment, object: id, key: id) {
    let key_str = ns_string::to_rust_string(env, key).into_owned();
    let observers = matching(env, object, &key_str);
    if observers.is_empty() {
        return;
    }
    let old = if observers
        .iter()
        .any(|&(_, options, _)| options & NSKeyValueObservingOptionOld != 0)
    {
        current_value(env, object, &key_str)
    } else {
        nil
    };
    for (observer, options, context) in observers {
        if options & NSKeyValueObservingOptionPrior != 0 {
            let change = change_dict(env, object, &key_str, options, old, true);
            notify(env, observer, &key_str, object, change, context);
        }
    }
    crate::objc::retain(env, old);
    State::get(env).old_values.push((object, key_str, old));
}

/// `didChangeValueForKey:`
pub fn did_change(env: &mut Environment, object: id, key: id) {
    let key_str = ns_string::to_rust_string(env, key).into_owned();
    let state = State::get(env);
    let old = state
        .old_values
        .iter()
        .rposition(|(o, k, _)| *o == object && *k == key_str)
        .map(|i| state.old_values.remove(i).2)
        .unwrap_or(nil);
    autorelease(env, old);
    for (observer, options, context) in matching(env, object, &key_str) {
        let change = change_dict(env, object, &key_str, options, old, false);
        notify(env, observer, &key_str, object, change, context);
    }
}

/// The value, or `NSNull` for nil as in a change dictionary.
fn current_value(env: &mut Environment, object: id, key: &str) -> id {
    let sel: SEL = env
        .objc
        .register_host_selector("valueForKey:".to_string(), &mut env.mem);
    let responds: bool = msg![env; object respondsToSelector:sel];
    let value = if responds {
        let key = ns_string::from_rust_string(env, key.to_string());
        let key = autorelease(env, key);
        msg![env; object valueForKey:key]
    } else {
        log_once!("TODO: KVO values for objects without valueForKey: are nil");
        nil
    };
    if value == nil {
        msg_class![env; NSNull null]
    } else {
        value
    }
}

fn change_dict(
    env: &mut Environment,
    object: id,
    key: &str,
    options: NSKeyValueObservingOptions,
    old: id,
    is_prior: bool,
) -> id {
    let dict: id = msg_class![env; NSMutableDictionary dictionary];
    let kind: id = msg_class![env; NSNumber numberWithUnsignedInt:NSKeyValueChangeSetting];
    let kind_key = ns_string::get_static_str(env, NSKeyValueChangeKindKey);
    () = msg![env; dict setObject:kind forKey:kind_key];
    if is_prior {
        let yes: id = msg_class![env; NSNumber numberWithBool:true];
        let prior_key = ns_string::get_static_str(env, NSKeyValueChangeNotificationIsPriorKey);
        () = msg![env; dict setObject:yes forKey:prior_key];
    } else if options & NSKeyValueObservingOptionNew != 0 {
        let new = current_value(env, object, key);
        let new_key = ns_string::get_static_str(env, NSKeyValueChangeNewKey);
        () = msg![env; dict setObject:new forKey:new_key];
    }
    if old != nil && options & NSKeyValueObservingOptionOld != 0 {
        let old_key = ns_string::get_static_str(env, NSKeyValueChangeOldKey);
        () = msg![env; dict setObject:old forKey:old_key];
    }
    dict
}

fn notify(
    env: &mut Environment,
    observer: id,
    key: &str,
    object: id,
    change: id,
    context: MutVoidPtr,
) {
    let key_path = ns_string::from_rust_string(env, key.to_string());
    let key_path = autorelease(env, key_path);
    () = msg![env; observer observeValueForKeyPath:key_path
                                          ofObject:object
                                            change:change
                                           context:context];
}
