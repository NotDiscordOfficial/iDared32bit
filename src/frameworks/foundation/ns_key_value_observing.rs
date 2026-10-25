/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! NSObject key-value observer registrations and setting notifications.

use super::{ns_string, NSUInteger};
use crate::mem::MutVoidPtr;
use crate::objc::{id, msg, msg_class, release, retain};
use crate::Environment;
use std::collections::HashMap;

const NEW: NSUInteger = 1;
const OLD: NSUInteger = 2;
const INITIAL: NSUInteger = 4;

#[derive(Clone)]
struct Observation {
    observer: id,
    key_path: String,
    options: NSUInteger,
    context: MutVoidPtr,
}

#[derive(Default)]
pub struct State {
    observations: HashMap<id, Vec<Observation>>,
}

pub fn add_observer(
    env: &mut Environment,
    object: id,
    observer: id,
    key_path: id,
    options: NSUInteger,
    context: MutVoidPtr,
) {
    let key_path = ns_string::to_rust_string(env, key_path).into_owned();
    retain(env, observer);
    env.framework_state
        .foundation
        .ns_key_value_observing
        .observations
        .entry(object)
        .or_default()
        .push(Observation {
            observer,
            key_path: key_path.clone(),
            options,
            context,
        });

    if options & INITIAL != 0 {
        // NSOperation's boolean state properties are the first users of this
        // facility. Their KVC values are NSNumber instances, as required by
        // the Objective-C observable-property contract.
        let class = msg![env; object class];
        let operation_class = env.objc.get_known_class("NSOperation", &mut env.mem);
        let current_value: Option<bool> =
            if env.objc.class_is_subclass_of(class, operation_class) {
            match key_path.as_str() {
                "isCancelled" => Some(msg![env; object isCancelled]),
                "isExecuting" => Some(msg![env; object isExecuting]),
                "isFinished" => Some(msg![env; object isFinished]),
                "isReady" => Some(msg![env; object isReady]),
                _ => None,
            }
        } else {
            None
        };
        if let Some(value) = current_value {
            let value: id = msg_class![env; NSNumber numberWithBool:value];
            notify(env, object, &key_path, nil_id(), value, options | NEW);
        }
    }
}

fn nil_id() -> id {
    crate::objc::nil
}

pub fn remove_observer(
    env: &mut Environment,
    object: id,
    observer: id,
    key_path: Option<&str>,
    context: Option<MutVoidPtr>,
) {
    let removed = {
        let state = &mut env.framework_state.foundation.ns_key_value_observing;
        let Some(observations) = state.observations.get_mut(&object) else {
            return;
        };
        let mut removed = Vec::new();
        observations.retain(|entry| {
            let matches = entry.observer == observer
                && key_path.is_none_or(|path| entry.key_path == path)
                && context.is_none_or(|ctx| entry.context == ctx);
            if matches {
                removed.push(entry.observer);
            }
            !matches
        });
        if observations.is_empty() {
            state.observations.remove(&object);
        }
        removed
    };
    for observer in removed {
        release(env, observer);
    }
}

pub fn remove_all_for_object(env: &mut Environment, object: id) {
    let observations = env
        .framework_state
        .foundation
        .ns_key_value_observing
        .observations
        .remove(&object)
        .unwrap_or_default();
    for observation in observations {
        release(env, observation.observer);
    }
}

pub fn notify_bool(
    env: &mut Environment,
    object: id,
    key_path: &str,
    old_value: bool,
    new_value: bool,
) {
    let old_value: id = msg_class![env; NSNumber numberWithBool:old_value];
    let new_value: id = msg_class![env; NSNumber numberWithBool:new_value];
    notify(env, object, key_path, old_value, new_value, NEW | OLD);
}

fn notify(
    env: &mut Environment,
    object: id,
    key_path: &str,
    old_value: id,
    new_value: id,
    _options: NSUInteger,
) {
    let observations = env
        .framework_state
        .foundation
        .ns_key_value_observing
        .observations
        .get(&object)
        .cloned()
        .unwrap_or_default();
    let matching: Vec<_> = observations
        .into_iter()
        .filter(|entry| entry.key_path == key_path)
        .collect();
    for observation in matching {
        retain(env, observation.observer);
        let key_path_object = ns_string::from_rust_string(env, key_path.to_owned());
        let change: id = msg_class![env; NSMutableDictionary new];
        let kind_key = ns_string::get_static_str(env, "kind");
        let kind: id = msg_class![env; NSNumber numberWithInt:1];
        () = msg![env; change setObject:kind forKey:kind_key];
        if observation.options & NEW != 0 && new_value != crate::objc::nil {
            let new_key = ns_string::get_static_str(env, "new");
            () = msg![env; change setObject:new_value forKey:new_key];
        }
        if observation.options & OLD != 0 && old_value != crate::objc::nil {
            let old_key = ns_string::get_static_str(env, "old");
            () = msg![env; change setObject:old_value forKey:old_key];
        }
        let selector = env
            .objc
            .lookup_selector("observeValueForKeyPath:ofObject:change:context:")
            .unwrap();
        let _: () = crate::objc::msg_send(
            env,
            (
                observation.observer,
                selector,
                key_path_object,
                object,
                change,
                observation.context,
            ),
        );
        release(env, change);
        release(env, key_path_object);
        release(env, observation.observer);
    }
}
