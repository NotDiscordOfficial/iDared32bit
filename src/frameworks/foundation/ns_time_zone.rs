/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSTimeZone`.

use crate::frameworks::foundation::{ns_string, NSInteger};
use crate::objc::{autorelease, id, nil, release, retain, ClassExports, HostObject, NSZonePtr};
use crate::{msg, objc_classes};

#[derive(Default)]
pub struct State {
    system_time_zone: Option<id>,
}

struct NSTimeZoneHostObject {
    // NSString*
    time_zone: id,
}
impl HostObject for NSTimeZoneHostObject {}

/// NSTimeZoneNameStyle values from Foundation (iOS 2+).
fn localized_gmt_name(style: NSInteger) -> Option<&'static str> {
    match style {
        0 | 2 | 4 => Some("Greenwich Mean Time"),
        1 | 3 | 5 => Some("GMT"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmt_localized_name_styles() {
        for style in [0, 2, 4] {
            assert_eq!(localized_gmt_name(style), Some("Greenwich Mean Time"));
        }
        for style in [1, 3, 5] {
            assert_eq!(localized_gmt_name(style), Some("GMT"));
        }
        assert_eq!(localized_gmt_name(6), None);
    }
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSTimeZone: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::new(NSTimeZoneHostObject {
        time_zone: nil,
    });
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

+ (id)timeZoneWithName:(id)tz_name {
    let new: id = msg![env; this alloc];
    let new: id = msg![env; new initWithName:tz_name];
    autorelease(env, new)
}

+ (id)localTimeZone {
    // According to docs, `localTimeZone` is not cached in contrast to
    // `systemTimeZone`
    let gmt_tz_name: id = ns_string::get_static_str(env, "GMT");
    msg![env; this timeZoneWithName:gmt_tz_name]
}

+ (id)systemTimeZone {
    if let Some(system_time_zone) = env.framework_state.foundation.ns_time_zone.system_time_zone {
        system_time_zone
    } else {
        let new: id = msg![env; this alloc];
        let gmt_tz_name: id = ns_string::get_static_str(env, "GMT");
        let new: id = msg![env; new initWithName:gmt_tz_name];
        env.framework_state.foundation.ns_time_zone.system_time_zone = Some(new);
        new
    }
}

+ (id)defaultTimeZone {
    // TODO: implement setting a default time zone
    msg![env; this systemTimeZone]
}

- (())dealloc {
    let tz_name = env.objc.borrow_mut::<NSTimeZoneHostObject>(this).time_zone;
    release(env, tz_name);
    env.objc.dealloc_object(this, &mut env.mem)
}

- (id)initWithName:(id)tz_name { // NSString *
    assert_ne!(tz_name, nil);
    retain(env, tz_name);
    env.objc.borrow_mut::<NSTimeZoneHostObject>(this).time_zone = tz_name;
    this
}

- (id)name {
    env.objc.borrow_mut::<NSTimeZoneHostObject>(this).time_zone
}

- (id)abbreviation {
    // TODO: support zone abbreviations
    ns_string::get_static_str(env, "GMT")
}

- (id)localizedName:(NSInteger)style locale:(id)_locale {
    // The emulator currently exposes only GMT/UTC timezone behavior.
    // Name styles 0/2/4 are long; 1/3/5 are abbreviated.
    let name = env.objc.borrow::<NSTimeZoneHostObject>(this).time_zone;
    let zone = ns_string::to_rust_string(env, name).into_owned();
    if zone == "GMT" || zone == "UTC" || zone == "Etc/GMT" {
        let Some(localized) = localized_gmt_name(style) else {
            return nil;
        };
        ns_string::get_static_str(env, localized)
    } else {
        // For other zones, preserve their name instead of inventing a
        // localized display name without time-zone/locale data.
        name
    }
}

- (NSInteger)secondsFromGMT {
    // TODO: respect timezone
    0
}

// NSCopying implementation
- (id)copyWithZone:(NSZonePtr)_zone {
    retain(env, this)
}

@end

};
