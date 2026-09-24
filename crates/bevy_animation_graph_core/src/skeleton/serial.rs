use bevy::{reflect::Reflect, transform::components::Transform};
use serde::{Deserialize, Serialize};

use crate::animation_clip::EntityPath;

#[derive(Serialize, Deserialize)]
pub struct SkeletonSerial {
    /// Where the bones come from
    pub source: SkeletonSource,
}

/// On the aurora branch a skeleton is baked, not walked out of a glTF scene at load time:
/// the importer reads the rig's `.bsn` and writes the bone list here, so the asset stands on
/// its own and the runtime never opens a glTF.
#[derive(Clone, Reflect, Serialize, Deserialize)]
pub enum SkeletonSource {
    Baked {
        /// Name-path of the bone the rest hang under (the armature root).
        root: EntityPath,
        /// Every bone, parents before children, with its rest pose.
        bones: Vec<BakedBone>,
    },
}

/// One bone's rest pose: `local` is relative to its parent, `character` is relative to the
/// armature root.
#[derive(Clone, Reflect, Serialize, Deserialize)]
pub struct BakedBone {
    pub path: EntityPath,
    pub local: Transform,
    pub character: Transform,
}
