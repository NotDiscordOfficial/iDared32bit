/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIPageControl`.

use crate::frameworks::core_graphics::cg_context::{
    CGContextFillEllipseInRect, CGContextSetRGBFillColor,
};
use crate::frameworks::core_graphics::{CGFloat, CGPoint, CGRect, CGSize};
use crate::frameworks::foundation::ns_string::get_static_str;
use crate::frameworks::foundation::NSInteger;
use crate::frameworks::uikit::ui_graphics::UIGraphicsGetCurrentContext;
use crate::frameworks::uikit::ui_view::ui_control::{
    send_actions, UIControlEventTouchUpInside, UIControlEventValueChanged,
};
use crate::objc::{
    id, impl_HostObject_with_superclass, msg, msg_super, objc_classes, ClassExports, NSZonePtr,
};
use crate::Environment;

const DOT_SIZE: CGFloat = 6.0;
/// Distance from the start of one dot to the start of the next.
const DOT_PITCH: CGFloat = 16.0;
const HEIGHT: CGFloat = 36.0;

#[derive(Default)]
pub struct UIPageControlHostObject {
    superclass: super::ui_control::UIControlHostObject,
    number_of_pages: NSInteger,
    current_page: NSInteger,
    /// The page the dots show, which lags behind `current_page` when
    /// `defersCurrentPageDisplay` is set.
    displayed_page: NSInteger,
    hides_for_single_page: bool,
    defers_current_page_display: bool,
}
impl_HostObject_with_superclass!(UIPageControlHostObject);

fn init_common(env: &mut Environment, this: id) {
    () = msg![env; this setOpaque:false];
    let selector = env
        .objc
        .lookup_selector("_touchHLE_tapAction:forEvent:")
        .unwrap();
    () = msg![env; this addTarget:this
                           action:selector
                 forControlEvents:UIControlEventTouchUpInside];
}

fn update(env: &mut Environment, this: id) {
    let host_obj = env.objc.borrow::<UIPageControlHostObject>(this);
    let hidden = host_obj.hides_for_single_page && host_obj.number_of_pages <= 1;
    () = msg![env; this setHidden:hidden];
    () = msg![env; this setNeedsDisplay];
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UIPageControl: UIControl

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<UIPageControlHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)initWithFrame:(CGRect)frame {
    let this: id = msg_super![env; this initWithFrame:frame];
    init_common(env, this);
    this
}

- (id)initWithCoder:(id)coder {
    let this: id = msg_super![env; this initWithCoder:coder];
    init_common(env, this);

    let key = get_static_str(env, "UINumberOfPages");
    let number_of_pages: NSInteger = msg![env; coder decodeIntegerForKey:key];
    let key = get_static_str(env, "UICurrentPage");
    let current_page: NSInteger = msg![env; coder decodeIntegerForKey:key];
    let key = get_static_str(env, "UIHidesForSinglePage");
    let hides_for_single_page: bool = msg![env; coder decodeBoolForKey:key];

    () = msg![env; this setHidesForSinglePage:hides_for_single_page];
    () = msg![env; this setNumberOfPages:number_of_pages];
    () = msg![env; this setCurrentPage:current_page];
    this
}

- (NSInteger)numberOfPages {
    env.objc.borrow::<UIPageControlHostObject>(this).number_of_pages
}
- (())setNumberOfPages:(NSInteger)number_of_pages {
    let host_obj = env.objc.borrow_mut::<UIPageControlHostObject>(this);
    host_obj.number_of_pages = number_of_pages.max(0);
    // The current page is kept within the new range.
    let last = (host_obj.number_of_pages - 1).max(0);
    host_obj.current_page = host_obj.current_page.min(last);
    host_obj.displayed_page = host_obj.displayed_page.min(last);
    update(env, this);
}

- (NSInteger)currentPage {
    env.objc.borrow::<UIPageControlHostObject>(this).current_page
}
- (())setCurrentPage:(NSInteger)current_page {
    let host_obj = env.objc.borrow_mut::<UIPageControlHostObject>(this);
    let last = (host_obj.number_of_pages - 1).max(0);
    host_obj.current_page = current_page.clamp(0, last);
    host_obj.displayed_page = host_obj.current_page;
    update(env, this);
}

- (bool)hidesForSinglePage {
    env.objc.borrow::<UIPageControlHostObject>(this).hides_for_single_page
}
- (())setHidesForSinglePage:(bool)hides {
    env.objc.borrow_mut::<UIPageControlHostObject>(this).hides_for_single_page = hides;
    update(env, this);
}

- (bool)defersCurrentPageDisplay {
    env.objc.borrow::<UIPageControlHostObject>(this).defers_current_page_display
}
- (())setDefersCurrentPageDisplay:(bool)defers {
    let host_obj = env.objc.borrow_mut::<UIPageControlHostObject>(this);
    host_obj.defers_current_page_display = defers;
}

- (())updateCurrentPageDisplay {
    let host_obj = env.objc.borrow_mut::<UIPageControlHostObject>(this);
    host_obj.displayed_page = host_obj.current_page;
    update(env, this);
}

- (CGSize)sizeForNumberOfPages:(NSInteger)page_count {
    CGSize {
        width: page_count.max(0) as CGFloat * DOT_PITCH,
        height: HEIGHT,
    }
}

- (())drawRect:(CGRect)_rect {
    let bounds: CGRect = msg![env; this bounds];
    let host_obj = env.objc.borrow::<UIPageControlHostObject>(this);
    let (count, displayed) = (host_obj.number_of_pages, host_obj.displayed_page);
    let size: CGSize = msg![env; this sizeForNumberOfPages:count];
    // The dots are centered, with the gap after the last dot left out.
    let width = size.width - (DOT_PITCH - DOT_SIZE);
    let x = bounds.origin.x + (bounds.size.width - width) / 2.0;
    let y = bounds.origin.y + (bounds.size.height - DOT_SIZE) / 2.0;

    let context = UIGraphicsGetCurrentContext(env);
    for i in 0..count {
        let alpha = if i == displayed { 1.0 } else { 0.4 };
        CGContextSetRGBFillColor(env, context, 1.0, 1.0, 1.0, alpha);
        let rect = CGRect {
            origin: CGPoint {
                x: x + i as CGFloat * DOT_PITCH,
                y,
            },
            size: CGSize {
                width: DOT_SIZE,
                height: DOT_SIZE,
            },
        };
        CGContextFillEllipseInRect(env, context, rect);
    }
}

// A tap on either side of the current page's dot goes one page that way.
- (())_touchHLE_tapAction:(id)sender forEvent:(id)event { // UIEvent*
    assert_eq!(this, sender);

    let touches: id = msg![env; event allTouches];
    let touch: id = msg![env; touches anyObject];
    let location: CGPoint = msg![env; touch locationInView:this];
    let bounds: CGRect = msg![env; this bounds];
    let host_obj = env.objc.borrow::<UIPageControlHostObject>(this);
    let (count, current) = (host_obj.number_of_pages, host_obj.current_page);
    let size: CGSize = msg![env; this sizeForNumberOfPages:count];
    let width = size.width - (DOT_PITCH - DOT_SIZE);
    let x = bounds.origin.x + (bounds.size.width - width) / 2.0;
    let dot_center = x + current as CGFloat * DOT_PITCH + DOT_SIZE / 2.0;

    let new_page = if location.x < dot_center {
        current - 1
    } else {
        current + 1
    };
    if new_page < 0 || new_page >= count {
        return;
    }

    let host_obj = env.objc.borrow_mut::<UIPageControlHostObject>(this);
    host_obj.current_page = new_page;
    if !host_obj.defers_current_page_display {
        host_obj.displayed_page = new_page;
    }
    update(env, this);
    send_actions(env, this, event, UIControlEventValueChanged);
}

@end

};
