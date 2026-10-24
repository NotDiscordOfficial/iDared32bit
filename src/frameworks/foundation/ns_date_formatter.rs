/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSDateFormatter`.
//!
//! Resources:
//! - Apple's [Introduction to Data Formatting Programming Guide For Cocoa](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/DataFormatting/DataFormatting.html)
//! - [Unicode Technical Standard #35](https://unicode.org/reports/tr35/tr35-10.html#Date_Format_Patterns)

use crate::frameworks::core_foundation::time::{CFAbsoluteTimeGetGregorianDate, CFGregorianDate};
use crate::frameworks::foundation::{ns_string, NSTimeInterval, NSUInteger};
use crate::objc::{
    autorelease, id, msg, nil, objc_classes, release, todo_objc_setter, ClassExports, HostObject,
    NSZonePtr,
};
use crate::Environment;

type NSDateFormatterStyle = NSUInteger;

struct NSDateFormatterHostObject {
    date_format: Option<id>,
    date_style: NSDateFormatterStyle,
    time_style: NSDateFormatterStyle,
}
impl HostObject for NSDateFormatterHostObject {}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSDateFormatter: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::new(NSDateFormatterHostObject {
        date_format: None,
        date_style: 0,
        time_style: 0,
    });
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (())setDateFormat:(id)format { // NSString *
    let date_format: id = msg![env; format copy];
    env.objc.borrow_mut::<NSDateFormatterHostObject>(this).date_format = Some(date_format);
}

- (())setTimeZone:(id)time_zone {
    todo_objc_setter!(this, time_zone);
}

// Setting a style replaces the format, as in Cocoa.
- (())setDateStyle:(NSDateFormatterStyle)style {
    env.objc.borrow_mut::<NSDateFormatterHostObject>(this).date_style = style;
    set_format_from_styles(env, this);
}
- (())setTimeStyle:(NSDateFormatterStyle)style {
    env.objc.borrow_mut::<NSDateFormatterHostObject>(this).time_style = style;
    set_format_from_styles(env, this);
}

- (id)stringFromDate:(id)date {
    let &NSDateFormatterHostObject {
        date_format, ..
    } = env.objc.borrow(this);
    let format = ns_string::to_rust_string(env, date_format.unwrap()).to_string();
    log_dbg!("date_format before: {:?}", format);

    let ti: NSTimeInterval = msg![env; date timeIntervalSinceReferenceDate];
    let greg_date = CFAbsoluteTimeGetGregorianDate(env, ti, nil);
    // The reference date (2001-01-01) was a Monday.
    let weekday = ((ti / 86400.0).floor() as i64 + 1).rem_euclid(7) as usize;
    let res = format_date(&format, &greg_date, weekday);
    log_dbg!("date_format after: {:?}", res);

    let res = ns_string::from_rust_string(env, res);
    autorelease(env, res)
}

@end

};

/// The en_US patterns for each style (none, short, medium, long, full).
fn set_format_from_styles(env: &mut Environment, this: id) {
    const DATE: [&str; 5] = [
        "",
        "M/d/yy",
        "MMM d, yyyy",
        "MMMM d, yyyy",
        "EEEE, MMMM d, yyyy",
    ];
    const TIME: [&str; 5] = ["", "h:mm a", "h:mm:ss a", "h:mm:ss a z", "h:mm:ss a zzzz"];
    let host_object = env.objc.borrow::<NSDateFormatterHostObject>(this);
    let date = DATE[host_object.date_style as usize];
    let time = TIME[host_object.time_style as usize];
    let format = [date, time]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let format = ns_string::from_rust_string(env, format);
    let old = env
        .objc
        .borrow_mut::<NSDateFormatterHostObject>(this)
        .date_format
        .replace(format);
    if let Some(old) = old {
        release(env, old);
    }
}

/// Formats a date with a Unicode date pattern (en_US, in UTC).
fn format_date(format: &str, date: &CFGregorianDate, weekday: usize) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    const DAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    let month = MONTHS[date.month as usize - 1];
    let hour12 = (date.hours as i32 + 11) % 12 + 1;
    let mut out = String::new();
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if !c.is_ascii_alphabetic() {
            if c == '\'' {
                // Quoted text, where '' is a quote.
                while let Some(c) = chars.next() {
                    if c == '\'' && chars.next_if_eq(&'\'').is_none() {
                        break;
                    }
                    out.push(c);
                }
            } else {
                out.push(c);
            }
            continue;
        }
        let mut n = 1;
        while chars.next_if_eq(&c).is_some() {
            n += 1;
        }
        let num = |v: i32| format!("{v:0n$}");
        out += &match c {
            'y' | 'Y' if n == 2 => format!("{:02}", date.year % 100),
            'y' | 'Y' => num(date.year),
            'M' if n >= 4 => month.to_string(),
            'M' if n == 3 => month[..3].to_string(),
            'M' => num(date.month.into()),
            'd' => num(date.day.into()),
            'H' => num(date.hours.into()),
            'h' => num(hour12),
            'm' => num(date.minutes.into()),
            's' => num(date.seconds as i32),
            'a' => (if date.hours < 12 { "AM" } else { "PM" }).to_string(),
            'E' if n >= 4 => DAYS[weekday].to_string(),
            'E' => DAYS[weekday][..3].to_string(),
            // Dates are worked out in UTC (see CFAbsoluteTimeGetGregorianDate).
            'z' if n >= 4 => "Greenwich Mean Time".to_string(),
            'z' => "GMT".to_string(),
            _ => unimplemented!("date format pattern {}", c.to_string().repeat(n)),
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_date() {
        // 2009-07-04 15:05:09, a Saturday.
        let date = CFGregorianDate {
            year: 2009,
            month: 7,
            day: 4,
            hours: 15,
            minutes: 5,
            seconds: 9.0,
        };
        let f = |format| format_date(format, &date, 6);
        assert_eq!(f("yyyy-MM-dd HH:mm:ss"), "2009-07-04 15:05:09");
        assert_eq!(f("M/d/yy h:mm a"), "7/4/09 3:05 PM");
        assert_eq!(f("MMM d, yyyy h:mm:ss a"), "Jul 4, 2009 3:05:09 PM");
        assert_eq!(f("EEEE, MMMM d, yyyy"), "Saturday, July 4, 2009");
        assert_eq!(f("'at' HH 'o''clock'"), "at 15 o'clock");
    }
}
