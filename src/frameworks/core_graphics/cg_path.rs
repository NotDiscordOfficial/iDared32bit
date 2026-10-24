/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `CGPath`
use super::cg_affine_transform::CGAffineTransform;
use super::CGPoint;
use crate::dyld::{export_c_func, FunctionExports};
use crate::frameworks::core_foundation::{CFRelease, CFRetain, CFTypeRef};
use crate::mem::{ConstPtr, GuestUSize};
use crate::objc::{objc_classes, ClassExports, HostObject};
use crate::Environment;

// Note: on iOS SDK side this type is defined as a pointer to an opaque struct
pub type CGMutablePathRef = CFTypeRef;

#[derive(Clone)]
pub struct CGMutablePathHostObject {
    pub path_elements: Vec<CGPathElement>,
}
impl HostObject for CGMutablePathHostObject {}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation _touchHLE_CGMutablePath: NSObject
@end

};

pub type CGPathElementType = i32;
pub const kCGPathElementMoveToPoint: CGPathElementType = 0;
pub const kCGPathElementAddLineToPoint: CGPathElementType = 1;

pub type CGLineCap = i32;
pub const kCGLineCapButt: CGLineCap = 0; // Default
pub const kCGLineCapRound: CGLineCap = 1;
pub const kCGLineCapSquare: CGLineCap = 2;

#[derive(Clone)]
pub struct CGPathElement {
    pub points: Vec<CGPoint>,
    pub r#type: CGPathElementType,
}

pub fn CGPathCreateMutable(env: &mut Environment) -> CGMutablePathRef {
    let path_elements: Vec<CGPathElement> = Vec::new();
    let host_obj = Box::new(CGMutablePathHostObject { path_elements });
    let class = env
        .objc
        .get_known_class("_touchHLE_CGMutablePath", &mut env.mem);
    env.objc.alloc_object(class, host_obj, &mut env.mem)
}

pub fn CGPathRetain(env: &mut Environment, path: CGMutablePathRef) -> CGMutablePathRef {
    if !path.is_null() {
        CFRetain(env, path)
    } else {
        path
    }
}
pub fn CGPathRelease(env: &mut Environment, path: CGMutablePathRef) {
    if !path.is_null() {
        CFRelease(env, path);
    }
}

/// Apply the optional transform that CGPath's adding functions take.
fn transformed(env: &Environment, m: ConstPtr<CGAffineTransform>, point: CGPoint) -> CGPoint {
    if m.is_null() {
        point
    } else {
        env.mem.read(m).apply_to_point(point)
    }
}

fn add_element(
    env: &mut Environment,
    path: CGMutablePathRef,
    type_: CGPathElementType,
    point: CGPoint,
) {
    env.objc
        .borrow_mut::<CGMutablePathHostObject>(path)
        .path_elements
        .push(CGPathElement {
            points: vec![point],
            r#type: type_,
        });
}

pub fn CGPathMoveToPoint(
    env: &mut Environment,
    path: CGMutablePathRef,
    m: ConstPtr<CGAffineTransform>,
    x: f32,
    y: f32,
) {
    let point = transformed(env, m, CGPoint { x, y });
    add_element(env, path, kCGPathElementMoveToPoint, point);
}

pub fn CGPathAddLineToPoint(
    env: &mut Environment,
    path: CGMutablePathRef,
    m: ConstPtr<CGAffineTransform>,
    x: f32,
    y: f32,
) {
    let point = transformed(env, m, CGPoint { x, y });
    add_element(env, path, kCGPathElementAddLineToPoint, point);
}

/// Like Apple's: a move to the first point, then lines to the others.
pub fn CGPathAddLines(
    env: &mut Environment,
    path: CGMutablePathRef,
    m: ConstPtr<CGAffineTransform>,
    points: ConstPtr<CGPoint>,
    count: GuestUSize,
) {
    for i in 0..count {
        let point = transformed(env, m, env.mem.read(points + i));
        let type_ = if i == 0 {
            kCGPathElementMoveToPoint
        } else {
            kCGPathElementAddLineToPoint
        };
        add_element(env, path, type_, point);
    }
}

/// The points of a path made only of moves and lines, for keyframe
/// animations. Panics on other elements, which aren't supported yet.
pub fn path_points(env: &Environment, path: CGMutablePathRef) -> Vec<CGPoint> {
    let host_obj = env.objc.borrow::<CGMutablePathHostObject>(path);
    host_obj
        .path_elements
        .iter()
        .map(|elem| {
            assert!(matches!(
                elem.r#type,
                kCGPathElementMoveToPoint | kCGPathElementAddLineToPoint
            ));
            elem.points[0]
        })
        .collect()
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(CGPathCreateMutable()),
    export_c_func!(CGPathRetain(_)),
    export_c_func!(CGPathRelease(_)),
    export_c_func!(CGPathMoveToPoint(_, _, _, _)),
    export_c_func!(CGPathAddLineToPoint(_, _, _, _)),
    export_c_func!(CGPathAddLines(_, _, _, _)),
];
