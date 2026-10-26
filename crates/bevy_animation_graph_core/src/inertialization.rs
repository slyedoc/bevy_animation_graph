//! Inertial blending (Bollo, "Inertialization", GDC 2018): instead of crossfading two live poses,
//! a transition plays only the target and adds the offset between the last pose actually output
//! and the target, decayed to zero by a quintic that starts at the offset's position, velocity
//! and acceleration and ends flat.

use bevy::{
    math::{Quat, Vec3},
    platform::collections::HashMap,
    reflect::Reflect,
};

use crate::pose::{BoneId, Pose};

/// One scalar offset decaying to zero over `t1`.
#[derive(Reflect, Debug, Clone, Copy, Default, PartialEq)]
pub struct Decay {
    x0: f32,
    v0: f32,
    a0: f32,
    t1: f32,
    a: f32,
    b: f32,
    c: f32,
}

impl Decay {
    /// `x0` is the offset (>= 0), `v0` its rate of change, `duration` the longest it may take.
    pub fn new(x0: f32, v0: f32, duration: f32) -> Self {
        let mut t1 = duration.max(0.);
        if v0 < 0. {
            t1 = t1.min(-5. * x0 / v0);
        }
        if t1 <= 1e-5 || (x0.abs() <= 1e-7 && v0 == 0.) {
            return Self::default();
        }
        let a0 = ((-8. * v0 * t1 - 20. * x0) / (t1 * t1)).max(0.);
        let (t2, t3) = (t1 * t1, t1 * t1 * t1);
        Self {
            x0,
            v0,
            a0,
            t1,
            a: -(a0 * t2 + 6. * v0 * t1 + 12. * x0) / (2. * t3 * t2),
            b: (3. * a0 * t2 + 16. * v0 * t1 + 30. * x0) / (2. * t2 * t2),
            c: -(3. * a0 * t2 + 12. * v0 * t1 + 20. * x0) / (2. * t3),
        }
    }

    pub fn eval(&self, t: f32) -> f32 {
        if t >= self.t1 {
            return 0.;
        }
        let t = t.max(0.);
        ((((self.a * t + self.b) * t + self.c) * t + 0.5 * self.a0) * t + self.v0) * t + self.x0
    }

    pub fn duration(&self) -> f32 {
        self.t1
    }
}

/// A vector offset: a direction and the decay of its length along it.
#[derive(Reflect, Debug, Clone, Copy, Default, PartialEq)]
pub struct VecOffset {
    dir: Vec3,
    decay: Decay,
}

impl VecOffset {
    fn new(target: Vec3, last: Vec3, prev: Option<Vec3>, dt: f32, duration: f32) -> Option<Self> {
        let offset = last - target;
        let x0 = offset.length();
        if x0 <= 1e-7 {
            return None;
        }
        let dir = offset / x0;
        let v0 = match prev {
            Some(prev) if dt > 0. => (x0 - (prev - target).dot(dir)) / dt,
            _ => 0.,
        };
        Some(Self {
            dir,
            decay: Decay::new(x0, v0, duration),
        })
    }

    fn eval(&self, t: f32) -> Vec3 {
        self.dir * self.decay.eval(t)
    }
}

/// A rotation offset, applied in the bone's parent space: an axis and the decay of the angle.
#[derive(Reflect, Debug, Clone, Copy, Default, PartialEq)]
pub struct RotOffset {
    axis: Vec3,
    decay: Decay,
}

impl RotOffset {
    fn new(target: Quat, last: Quat, prev: Option<Quat>, dt: f32, duration: f32) -> Option<Self> {
        let mut q0 = (last * target.inverse()).normalize();
        if q0.w < 0. {
            q0 = -q0;
        }
        let xyz = Vec3::new(q0.x, q0.y, q0.z);
        let sin_half = xyz.length();
        if sin_half <= 1e-6 {
            return None;
        }
        let axis = xyz / sin_half;
        let x0 = 2. * sin_half.atan2(q0.w);
        let v0 = match prev {
            Some(prev) if dt > 0. => {
                let mut q1 = (prev * target.inverse()).normalize();
                if q1.dot(q0) < 0. {
                    q1 = -q1;
                }
                let x1 = 2. * Vec3::new(q1.x, q1.y, q1.z).dot(axis).atan2(q1.w);
                (x0 - x1) / dt
            }
            _ => 0.,
        };
        Some(Self {
            axis,
            decay: Decay::new(x0, v0, duration),
        })
    }

    fn eval(&self, t: f32) -> Quat {
        Quat::from_axis_angle(self.axis, self.decay.eval(t))
    }
}

#[derive(Reflect, Debug, Clone, Default, PartialEq)]
pub struct BoneOffset {
    translation: Option<VecOffset>,
    rotation: Option<RotOffset>,
    scale: Option<VecOffset>,
    weights: Option<Vec<(f32, Decay)>>,
}

/// The offsets that carry a pose that was being output into a new target pose.
#[derive(Reflect, Debug, Clone, Default, PartialEq)]
pub struct InertialPose {
    bones: HashMap<BoneId, BoneOffset>,
}

impl InertialPose {
    /// `last` and `prev` are the two most recent poses output, `dt` the time between them;
    /// `target` is the pose to blend into, sampled at the same instant as `last`.
    pub fn capture(
        target: &Pose,
        last: &Pose,
        prev: Option<&Pose>,
        dt: f32,
        duration: f32,
    ) -> Self {
        let mut bones = HashMap::default();
        for (bone_id, &index) in target.paths.iter() {
            let Some(last_bone) = last.get_bone(*bone_id) else {
                continue;
            };
            let target_bone = &target.bones[index];
            let prev_bone = prev.and_then(|p| p.get_bone(*bone_id));
            let offset = BoneOffset {
                translation: target_bone.translation.zip(last_bone.translation).and_then(
                    |(t, l)| {
                        VecOffset::new(t, l, prev_bone.and_then(|p| p.translation), dt, duration)
                    },
                ),
                rotation: target_bone
                    .rotation
                    .zip(last_bone.rotation)
                    .and_then(|(t, l)| {
                        RotOffset::new(t, l, prev_bone.and_then(|p| p.rotation), dt, duration)
                    }),
                scale: target_bone.scale.zip(last_bone.scale).and_then(|(t, l)| {
                    VecOffset::new(t, l, prev_bone.and_then(|p| p.scale), dt, duration)
                }),
                weights: target_bone
                    .weights
                    .as_ref()
                    .zip(last_bone.weights.as_ref())
                    .filter(|(t, l)| t.len() == l.len())
                    .map(|(t, l)| {
                        let p = prev_bone
                            .and_then(|p| p.weights.as_ref())
                            .filter(|p| p.len() == t.len());
                        t.iter()
                            .zip(l)
                            .enumerate()
                            .map(|(i, (t, l))| {
                                let x = l - t;
                                let sign = if x < 0. { -1. } else { 1. };
                                let v0 = match p {
                                    Some(p) if dt > 0. => sign * (l - p[i]) / dt,
                                    _ => 0.,
                                };
                                (sign, Decay::new(x.abs(), v0, duration))
                            })
                            .collect()
                    }),
            };
            if offset != BoneOffset::default() {
                bones.insert(*bone_id, offset);
            }
        }
        Self { bones }
    }

    /// Adds the offsets, `t` seconds after the capture, onto the target pose.
    pub fn apply(&self, pose: &mut Pose, t: f32) {
        for (bone_id, offset) in self.bones.iter() {
            let Some(&index) = pose.paths.get(bone_id) else {
                continue;
            };
            let bone = &mut pose.bones[index];
            if let (Some(o), Some(v)) = (&offset.translation, bone.translation.as_mut()) {
                *v += o.eval(t);
            }
            if let (Some(o), Some(q)) = (&offset.rotation, bone.rotation.as_mut()) {
                *q = (o.eval(t) * *q).normalize();
            }
            if let (Some(o), Some(v)) = (&offset.scale, bone.scale.as_mut()) {
                *v += o.eval(t);
            }
            if let (Some(o), Some(w)) = (&offset.weights, bone.weights.as_mut()) {
                for ((sign, decay), w) in o.iter().zip(w.iter_mut()) {
                    *w += sign * decay.eval(t);
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.bones.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use super::*;
    use crate::pose::BonePose;

    fn pose(translation: Vec3, rotation: Quat) -> Pose {
        let mut pose = Pose::default();
        pose.add_bone(
            BonePose {
                translation: Some(translation),
                rotation: Some(rotation),
                ..Default::default()
            },
            BoneId::default(),
        );
        pose
    }

    #[test]
    fn decay_starts_at_offset_and_ends_flat() {
        let d = Decay::new(1., 0., 0.5);
        assert!((d.eval(0.) - 1.).abs() < 1e-5);
        assert!(d.eval(0.25) > 0. && d.eval(0.25) < 1.);
        assert!(d.eval(0.4999).abs() < 1e-3);
        assert_eq!(d.eval(0.5), 0.);
        let mut last = d.eval(0.);
        for i in 1..=50 {
            let x = d.eval(i as f32 * 0.01);
            assert!(x <= last + 1e-6 && x >= -1e-6);
            last = x;
        }
    }

    #[test]
    fn decay_carries_velocity_without_crossing() {
        let away = Decay::new(0.1, 2., 0.5);
        assert!(away.eval(0.05) > 0.1);
        let toward = Decay::new(0.1, -2., 0.5);
        assert!(toward.duration() <= 0.25 + 1e-6);
        for i in 0..=50 {
            assert!(away.eval(i as f32 * 0.01) >= -1e-5);
            assert!(toward.eval(i as f32 * 0.01) >= -1e-5);
        }
    }

    #[test]
    fn capture_reproduces_last_pose_then_reaches_target() {
        let target = pose(Vec3::new(0., 1., 0.), Quat::IDENTITY);
        let last = pose(Vec3::new(0., 0.5, 0.), Quat::from_rotation_x(FRAC_PI_2));
        let prev = pose(
            Vec3::new(0., 0.45, 0.),
            Quat::from_rotation_x(FRAC_PI_2 * 0.95),
        );
        let offsets = InertialPose::capture(&target, &last, Some(&prev), 1. / 60., 0.4);

        let mut at_start = target.clone();
        offsets.apply(&mut at_start, 0.);
        let bone = at_start.get_bone(BoneId::default()).unwrap();
        assert!((bone.translation.unwrap() - Vec3::new(0., 0.5, 0.)).length() < 1e-4);
        assert!(
            bone.rotation
                .unwrap()
                .dot(Quat::from_rotation_x(FRAC_PI_2))
                .abs()
                > 0.99999
        );

        let mut next = target.clone();
        offsets.apply(&mut next, 1. / 60.);
        let y = next
            .get_bone(BoneId::default())
            .unwrap()
            .translation
            .unwrap()
            .y;
        assert!(y > 0.5 && y < 0.56, "velocity carries on: {y}");

        let mut at_end = target.clone();
        offsets.apply(&mut at_end, 0.4);
        assert_eq!(at_end, target);
    }
}
