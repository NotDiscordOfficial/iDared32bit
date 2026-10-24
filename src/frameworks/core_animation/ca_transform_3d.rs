/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `CATransform3D`. Layers only have 2D transforms here, so only the affine
//! part of one is used.

use crate::abi::{impl_GuestRet_for_large_struct, GuestArg};
use crate::dyld::{export_c_func, ConstantExports, FunctionExports, HostConstant};
use crate::frameworks::core_graphics::cg_affine_transform::{
    CGAffineTransform, CGAffineTransformIdentity,
};
use crate::frameworks::core_graphics::CGFloat;
use crate::mem::SafeRead;
use crate::Environment;

#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(C, packed)]
pub struct CATransform3D {
    pub m: [CGFloat; 16],
}
unsafe impl SafeRead for CATransform3D {}
impl GuestArg for CATransform3D {
    const REG_COUNT: usize = 16;

    fn from_regs(regs: &[u32]) -> Self {
        CATransform3D {
            m: std::array::from_fn(|i| GuestArg::from_regs(&regs[i..i + 1])),
        }
    }
    fn to_regs(self, regs: &mut [u32]) {
        let m = self.m;
        for (i, value) in m.into_iter().enumerate() {
            value.to_regs(&mut regs[i..i + 1]);
        }
    }
}
impl_GuestRet_for_large_struct!(CATransform3D);

impl From<CGAffineTransform> for CATransform3D {
    fn from(t: CGAffineTransform) -> Self {
        #[rustfmt::skip]
        let m = [
            t.a,  t.b,  0.0, 0.0,
            t.c,  t.d,  0.0, 0.0,
            0.0,  0.0,  1.0, 0.0,
            t.tx, t.ty, 0.0, 1.0,
        ];
        CATransform3D { m }
    }
}

impl CATransform3D {
    pub fn is_affine(&self) -> bool {
        let m = self.m;
        m[2] == 0.0
            && m[3] == 0.0
            && m[6] == 0.0
            && m[7] == 0.0
            && m[8] == 0.0
            && m[9] == 0.0
            && m[10] == 1.0
            && m[11] == 0.0
            && m[14] == 0.0
            && m[15] == 1.0
    }

    pub fn affine_part(&self) -> CGAffineTransform {
        let m = self.m;
        CGAffineTransform {
            a: m[0],
            b: m[1],
            c: m[4],
            d: m[5],
            tx: m[12],
            ty: m[13],
        }
    }

    /// `a` followed by `b`, like `CATransform3DConcat`.
    pub fn concat(a: Self, b: Self) -> Self {
        let (a, b) = (a.m, b.m);
        let m = std::array::from_fn(|i| {
            let (row, col) = (i / 4, i % 4);
            (0..4).map(|k| a[row * 4 + k] * b[k * 4 + col]).sum()
        });
        CATransform3D { m }
    }

    fn make_scale(sx: CGFloat, sy: CGFloat, sz: CGFloat) -> Self {
        let mut t = CATransform3D::from(CGAffineTransformIdentity);
        (t.m[0], t.m[5], t.m[10]) = (sx, sy, sz);
        t
    }

    fn make_translation(tx: CGFloat, ty: CGFloat, tz: CGFloat) -> Self {
        let mut t = CATransform3D::from(CGAffineTransformIdentity);
        (t.m[12], t.m[13], t.m[14]) = (tx, ty, tz);
        t
    }

    fn make_rotation(angle: CGFloat, x: CGFloat, y: CGFloat, z: CGFloat) -> Self {
        let len = (x * x + y * y + z * z).sqrt();
        let u = [x / len, y / len, z / len];
        let (s, c) = angle.sin_cos();
        let mut t = CATransform3D::from(CGAffineTransformIdentity);
        for i in 0..3 {
            for j in 0..3 {
                let mut value = (1.0 - c) * u[i] * u[j];
                if i == j {
                    value += c;
                } else {
                    // Sign of the Levi-Civita symbol for (i, j, k).
                    let k = 3 - i - j;
                    let sign = if (i + 1) % 3 == j { 1.0 } else { -1.0 };
                    value += sign * s * u[k];
                }
                t.m[i * 4 + j] = value;
            }
        }
        t
    }
}

fn CATransform3DMakeAffineTransform(_env: &mut Environment, t: CGAffineTransform) -> CATransform3D {
    t.into()
}

fn CATransform3DIsAffine(_env: &mut Environment, t: CATransform3D) -> bool {
    t.is_affine()
}

fn CATransform3DGetAffineTransform(_env: &mut Environment, t: CATransform3D) -> CGAffineTransform {
    t.affine_part()
}

fn CATransform3DMakeScale(
    _env: &mut Environment,
    sx: CGFloat,
    sy: CGFloat,
    sz: CGFloat,
) -> CATransform3D {
    CATransform3D::make_scale(sx, sy, sz)
}

fn CATransform3DScale(
    _env: &mut Environment,
    t: CATransform3D,
    sx: CGFloat,
    sy: CGFloat,
    sz: CGFloat,
) -> CATransform3D {
    CATransform3D::concat(CATransform3D::make_scale(sx, sy, sz), t)
}

fn CATransform3DTranslate(
    _env: &mut Environment,
    t: CATransform3D,
    tx: CGFloat,
    ty: CGFloat,
    tz: CGFloat,
) -> CATransform3D {
    CATransform3D::concat(CATransform3D::make_translation(tx, ty, tz), t)
}

fn CATransform3DRotate(
    _env: &mut Environment,
    t: CATransform3D,
    angle: CGFloat,
    x: CGFloat,
    y: CGFloat,
    z: CGFloat,
) -> CATransform3D {
    CATransform3D::concat(CATransform3D::make_rotation(angle, x, y, z), t)
}

pub const CONSTANTS: ConstantExports = &[(
    "_CATransform3DIdentity",
    HostConstant::Custom(|env| {
        let identity = CATransform3D::from(CGAffineTransformIdentity);
        env.mem.alloc_and_write(identity).cast().cast_const()
    }),
)];

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(CATransform3DMakeAffineTransform(_)),
    export_c_func!(CATransform3DIsAffine(_)),
    export_c_func!(CATransform3DGetAffineTransform(_)),
    export_c_func!(CATransform3DMakeScale(_, _, _)),
    export_c_func!(CATransform3DScale(_, _, _, _)),
    export_c_func!(CATransform3DTranslate(_, _, _, _)),
    export_c_func!(CATransform3DRotate(_, _, _, _, _)),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affine_round_trip() {
        let affine = CGAffineTransform {
            a: 0.5,
            b: -0.25,
            c: 0.25,
            d: 0.5,
            tx: 10.0,
            ty: 20.0,
        };
        let transform = CATransform3D::from(affine);
        assert!(transform.is_affine());
        assert_eq!(transform.affine_part(), affine);
        let mut perspective = transform;
        perspective.m[11] = -0.002;
        assert!(!perspective.is_affine());
    }

    #[test]
    fn rotation_matches_affine() {
        let angle: CGFloat = 0.5;
        let rotation = CATransform3D::make_rotation(angle, 0.0, 0.0, 2.0);
        let (s, c) = angle.sin_cos();
        let affine = CGAffineTransform {
            a: c,
            b: s,
            c: -s,
            d: c,
            tx: 0.0,
            ty: 0.0,
        };
        assert!(rotation.is_affine());
        assert_eq!(rotation.affine_part(), affine);
    }

    #[test]
    fn translate_then_scale() {
        let t = CATransform3D::make_translation(10.0, 20.0, 0.0);
        let scale = CATransform3D::make_scale(2.0, 3.0, 1.0);
        let affine = CATransform3D::concat(scale, t).affine_part();
        assert_eq!((affine.a, affine.d), (2.0, 3.0));
        assert_eq!((affine.tx, affine.ty), (10.0, 20.0));
    }
}
