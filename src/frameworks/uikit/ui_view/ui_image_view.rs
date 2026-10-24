/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIImageView`.

use crate::frameworks::core_animation::ca_animation::kCAAnimationDiscrete;
use crate::frameworks::core_graphics::cg_image::CGImageRef;
use crate::frameworks::core_graphics::{CGPoint, CGRect, CGSize};
use crate::frameworks::foundation::ns_string::get_static_str;
use crate::frameworks::foundation::{ns_array, NSInteger, NSTimeInterval, NSUInteger};
use crate::objc::{
    id, impl_HostObject_with_superclass, msg, msg_class, msg_super, nil, objc_classes, release,
    retain, ClassExports, NSZonePtr,
};

/// The layer animation key for `startAnimating`.
const ANIMATION_KEY: &str = "contents";

#[derive(Default)]
struct UIImageViewHostObject {
    superclass: super::UIViewHostObject,
    /// `UIImage*`
    image: id,
    /// `NSArray<UIImage*>*`
    animation_images: id,
    animation_duration: NSTimeInterval,
    animation_repeat_count: NSInteger,
}
impl_HostObject_with_superclass!(UIImageViewHostObject);

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UIImageView: UIView

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<UIImageViewHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)initWithFrame:(CGRect)frame {
    let this: id = msg_super![env; this initWithFrame:frame];
    // Not sure if UIImageView does this unconditionally, or only for images
    // with alpha channels.
    () = msg![env; this setOpaque:false];
    // Unlike UIView, touches go through by default.
    () = msg![env; this setUserInteractionEnabled:false];
    this
}

- (())dealloc {
    let &UIImageViewHostObject {
        superclass: _,
        image,
        animation_images,
        animation_duration: _,
        animation_repeat_count: _,
    } = env.objc.borrow(this);
    release(env, image);
    release(env, animation_images);
    msg_super![env; this dealloc]
}

// NSCoding implementation
- (id)initWithCoder:(id)coder {
    let this: id = msg_super![env; this initWithCoder:coder];

    let key_ns_string = get_static_str(env, "UIImage");
    let image: id = msg![env; coder decodeObjectForKey:key_ns_string];

    () = msg![env; this setImage:image];

    this
}

- (id)initWithImage:(id)image { // UIImage*
    let size: CGSize = msg![env; image size];
    let frame = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size
    };
    let this = msg_super![env; this initWithFrame:frame];
    () = msg![env; this setImage:image];
    // Not sure if UIImageView does this unconditionally, or only for images
    // with alpha channels.
    () = msg![env; this setOpaque:false];
    // Unlike UIView, touches go through by default.
    () = msg![env; this setUserInteractionEnabled:false];
    this
}

- (id)image {
    env.objc.borrow::<UIImageViewHostObject>(this).image
}

- (())setImage:(id)new_image { // UIImage*
    let host_obj = env.objc.borrow_mut::<UIImageViewHostObject>(this);
    let old_image = std::mem::replace(&mut host_obj.image, new_image);
    retain(env, new_image);
    release(env, old_image);

    let layer: id = msg![env; this layer];
    let cg_image: CGImageRef = msg![env; new_image CGImage];
    () = msg![env; layer setContents:cg_image];
}

- (id)animationImages {
    env.objc.borrow::<UIImageViewHostObject>(this).animation_images
}
- (())setAnimationImages:(id)images { // NSArray<UIImage *>*
    let images: id = msg![env; images copy];
    let host_obj = env.objc.borrow_mut::<UIImageViewHostObject>(this);
    let old = std::mem::replace(&mut host_obj.animation_images, images);
    release(env, old);
}

- (NSTimeInterval)animationDuration {
    env.objc.borrow::<UIImageViewHostObject>(this).animation_duration
}
- (())setAnimationDuration:(NSTimeInterval)duration {
    env.objc.borrow_mut::<UIImageViewHostObject>(this).animation_duration = duration;
}

- (NSInteger)animationRepeatCount {
    env.objc.borrow::<UIImageViewHostObject>(this).animation_repeat_count
}
- (())setAnimationRepeatCount:(NSInteger)count {
    env.objc.borrow_mut::<UIImageViewHostObject>(this).animation_repeat_count = count;
}

// Like UIKit, this shows the images with a keyframe animation of the
// layer's contents.
- (())startAnimating {
    let &UIImageViewHostObject {
        animation_images,
        animation_duration,
        animation_repeat_count,
        ..
    } = env.objc.borrow(this);
    let count: NSUInteger = msg![env; animation_images count];
    if count == 0 {
        return;
    }
    let mut cg_images = Vec::new();
    for i in 0..count {
        let image: id = msg![env; animation_images objectAtIndex:i];
        let cg_image: id = msg![env; image CGImage];
        // The array takes ownership of a reference.
        retain(env, cg_image);
        cg_images.push(cg_image);
    }
    let values = ns_array::from_vec(env, cg_images);

    // The default is 30 images a second.
    let duration = if animation_duration > 0.0 {
        animation_duration
    } else {
        count as NSTimeInterval / 30.0
    };
    // A repeat count of 0 means forever.
    let repeat_count = if animation_repeat_count > 0 {
        animation_repeat_count as f32
    } else {
        f32::MAX
    };

    let key = get_static_str(env, ANIMATION_KEY);
    let mode = get_static_str(env, kCAAnimationDiscrete);
    let animation: id = msg_class![env; CAKeyframeAnimation animationWithKeyPath:key];
    () = msg![env; animation setValues:values];
    () = msg![env; animation setCalculationMode:mode];
    () = msg![env; animation setDuration:duration];
    () = msg![env; animation setRepeatCount:repeat_count];
    release(env, values);

    let layer: id = msg![env; this layer];
    () = msg![env; layer addAnimation:animation forKey:key];
}

- (())stopAnimating {
    let key = get_static_str(env, ANIMATION_KEY);
    let layer: id = msg![env; this layer];
    () = msg![env; layer removeAnimationForKey:key];
}

- (bool)isAnimating {
    let key = get_static_str(env, ANIMATION_KEY);
    let layer: id = msg![env; this layer];
    let animation: id = msg![env; layer animationForKey:key];
    animation != nil
}

@end

};
