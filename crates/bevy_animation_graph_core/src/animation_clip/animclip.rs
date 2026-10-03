//! Reader for `.animclip`, the baked TRS-keyframe format the aurora stack uses in place of
//! glTF at runtime.
//!
//! The format is owned by `bevy_aurora::animclip` (which loads the same bytes into a stock
//! [`AnimationClip`] for `AnimationPlayer`); this is the same parse against
//! [`GraphClip`](super::GraphClip)'s curve store, so a graph and a stock player can share one
//! baked file. Targets are keyed by `AnimationTargetId::from_names` over the node's `Name`
//! chain, which is exactly [`EntityPath::id`](super::EntityPath::id).
//!
//! Layout (little-endian) — writers must match byte for byte:
//! ```text
//!   magic  "ANIMCLP\x01"                     (8 bytes)
//!   u32    target_count
//!   per target:
//!     u16  path_len;  per component: u16 len + len UTF-8 bytes (Name chain, top-level..node)
//!     u8   channel_mask                       (bit0 = translation, bit1 = rotation, bit2 = scale)
//!     per present channel, in T,R,S order:
//!       u32  key_count
//!       key_count x f32        times (seconds)
//!       key_count x dim x f32  values         (dim: T=3, R=4 [x,y,z,w], S=3)
//! ```
//! Every sampler is LINEAR.

use bevy::{
    animation::{
        AnimationTargetId, VariableCurve, animated_field, animation_curves::AnimatableCurve,
    },
    curve::{ConstantCurve, Interval, UnevenSampleAutoCurve},
    prelude::*,
};
use thiserror::Error;

const MAGIC: &[u8; 8] = b"ANIMCLP\x01";
const CH_T: u8 = 1;
const CH_R: u8 = 2;
const CH_S: u8 = 4;

#[derive(Debug, Error)]
pub enum AnimClipError {
    #[error("bad .animclip magic")]
    BadMagic,
    #[error("truncated .animclip")]
    Truncated,
    #[error("invalid UTF-8 in .animclip name path")]
    BadUtf8,
}

/// Parse `.animclip` bytes into a stock [`AnimationClip`].
pub fn parse_animclip(bytes: &[u8]) -> Result<AnimationClip, AnimClipError> {
    let mut c = Cur(bytes);
    if c.take(8)? != MAGIC {
        return Err(AnimClipError::BadMagic);
    }
    let mut clip = AnimationClip::default();
    let target_count = c.u32()?;
    for _ in 0..target_count {
        let parts = c.u16()?;
        let mut path: Vec<Name> = Vec::with_capacity(parts);
        for _ in 0..parts {
            let len = c.u16()?;
            let s = core::str::from_utf8(c.take(len)?).map_err(|_| AnimClipError::BadUtf8)?;
            path.push(Name::new(s.to_string()));
        }
        let target = AnimationTargetId::from_names(path.iter());

        let mask = c.u8()?;
        if mask & CH_T != 0 {
            let (times, vals) = read_channel(&mut c, 3)?;
            let pts: Vec<Vec3> = vals
                .chunks_exact(3)
                .map(|v| Vec3::new(v[0], v[1], v[2]))
                .collect();
            if let Some(vc) = translation_curve(&times, pts) {
                clip.add_variable_curve_to_target(target, vc);
            }
        }
        if mask & CH_R != 0 {
            let (times, vals) = read_channel(&mut c, 4)?;
            let pts: Vec<Quat> = vals
                .chunks_exact(4)
                .map(|v| Quat::from_array([v[0], v[1], v[2], v[3]]))
                .collect();
            if let Some(vc) = rotation_curve(&times, pts) {
                clip.add_variable_curve_to_target(target, vc);
            }
        }
        if mask & CH_S != 0 {
            let (times, vals) = read_channel(&mut c, 3)?;
            let pts: Vec<Vec3> = vals
                .chunks_exact(3)
                .map(|v| Vec3::new(v[0], v[1], v[2]))
                .collect();
            if let Some(vc) = scale_curve(&times, pts) {
                clip.add_variable_curve_to_target(target, vc);
            }
        }
    }
    Ok(clip)
}

struct Cur<'a>(&'a [u8]);

impl Cur<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], AnimClipError> {
        if self.0.len() < n {
            return Err(AnimClipError::Truncated);
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, AnimClipError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<usize, AnimClipError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as usize)
    }
    fn u32(&mut self) -> Result<usize, AnimClipError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()) as usize)
    }
    fn f32s(&mut self, n: usize) -> Result<Vec<f32>, AnimClipError> {
        let raw = self.take(n * 4)?;
        Ok(raw
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect())
    }
}

fn read_channel(c: &mut Cur, dim: usize) -> Result<(Vec<f32>, Vec<f32>), AnimClipError> {
    let keys = c.u32()?;
    let times = c.f32s(keys)?;
    let values = c.f32s(keys * dim)?;
    Ok((times, values))
}

// One builder per field: a single keyframe collapses to a `ConstantCurve` (as bevy_gltf does);
// LINEAR uses `UnevenSampleAutoCurve`, which slerps the quaternion field.
macro_rules! trs_curve {
    ($fn:ident, $field:ident, $ty:ty) => {
        fn $fn(times: &[f32], pts: Vec<$ty>) -> Option<VariableCurve> {
            if pts.is_empty() {
                return None;
            }
            if pts.len() == 1 {
                return Some(VariableCurve::new(AnimatableCurve::new(
                    animated_field!(Transform::$field),
                    ConstantCurve::new(Interval::EVERYWHERE, pts[0]),
                )));
            }
            UnevenSampleAutoCurve::new(times.iter().copied().zip(pts))
                .ok()
                .map(|curve| {
                    VariableCurve::new(AnimatableCurve::new(
                        animated_field!(Transform::$field),
                        curve,
                    ))
                })
        }
    };
}

trs_curve!(translation_curve, translation, Vec3);
trs_curve!(rotation_curve, rotation, Quat);
trs_curve!(scale_curve, scale, Vec3);
