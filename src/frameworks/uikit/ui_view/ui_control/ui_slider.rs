/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UISlider`.

use super::UIControlState;
use crate::frameworks::core_graphics::CGRect;
use crate::objc::{
    id, impl_HostObject_with_superclass, msg, msg_super, objc_classes, todo_objc_setter,
    ClassExports, NSZonePtr,
};

pub struct UISliderHostObject {
    superclass: super::UIControlHostObject,
    value: f32,
    minimum_value: f32,
    maximum_value: f32,
}
impl_HostObject_with_superclass!(UISliderHostObject);
impl Default for UISliderHostObject {
    fn default() -> Self {
        UISliderHostObject {
            superclass: Default::default(),
            value: 0.0,
            minimum_value: 0.0,
            maximum_value: 1.0,
        }
    }
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UISlider: UIControl

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<UISliderHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)initWithFrame:(CGRect)frame {
    log!("[(UISlider*){:?} initWithFrame:{:?}] TODO: Implement UISlider. The control won't be rendered.", this, frame);
    msg_super![env; this initWithFrame:frame]
}

// NSCoding implementation
- (id)initWithCoder:(id)coder {
    log!("[(UISlider*){:?} initWithCoder:{:?}] TODO: Implement UISlider. The control won't be rendered.", this, coder);
    msg_super![env; this initWithCoder:coder]
}

- (())setMinimumValueImage:(id)img { // UIImage *
    todo_objc_setter!(this, img);
}
- (())setMaximumValueImage:(id)img { // UIImage *
    todo_objc_setter!(this, img);
}

- (f32)minimumValue {
    env.objc.borrow::<UISliderHostObject>(this).minimum_value
}
- (())setMinimumValue:(f32)value {
    env.objc.borrow_mut::<UISliderHostObject>(this).minimum_value = value;
}

- (f32)maximumValue {
    env.objc.borrow::<UISliderHostObject>(this).maximum_value
}
- (())setMaximumValue:(f32)value {
    env.objc.borrow_mut::<UISliderHostObject>(this).maximum_value = value;
}

- (f32)value {
    env.objc.borrow::<UISliderHostObject>(this).value
}
- (())setValue:(f32)value {
    let host_object = env.objc.borrow_mut::<UISliderHostObject>(this);
    // Like Apple's, keep it between the minimum and maximum.
    host_object.value = value
        .min(host_object.maximum_value)
        .max(host_object.minimum_value);
}

- (())setValue:(f32)value animated:(bool)_animated {
    () = msg![env; this setValue:value];
}

- (())setThumbImage:(id)img
          forState:(UIControlState)state {
    log!("TODO: [(UISlider*){:?} setThumbImage:{:?} forState:{}]", this, img, state);
}

- (())setMinimumTrackImage:(id)img
                 forState:(UIControlState)state {
    log!("TODO: [(UISlider*){:?} setMinimumTrackImage:{:?} forState:{}]", this, img, state);
}

- (())setMaximumTrackImage:(id)img
                 forState:(UIControlState)state {
    log!("TODO: [(UISlider*){:?} setMaximumTrackImage:{:?} forState:{}]", this, img, state);
}

// TODO: all of it

@end

};
