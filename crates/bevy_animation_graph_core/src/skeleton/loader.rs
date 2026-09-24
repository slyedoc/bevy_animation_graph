use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    reflect::TypePath,
};

use super::{
    Skeleton,
    serial::{SkeletonSerial, SkeletonSource},
};
use crate::errors::AssetLoaderError;

#[derive(Default, TypePath)]
pub struct SkeletonLoader;

impl AssetLoader for SkeletonLoader {
    type Asset = Skeleton;
    type Settings = ();
    type Error = AssetLoaderError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = vec![];
        reader.read_to_end(&mut bytes).await?;
        let serial: SkeletonSerial = ron::de::from_bytes(&bytes)?;
        let skeleton: Skeleton = match serial.source {
            SkeletonSource::Baked { root, bones } => {
                let mut skeleton = Skeleton::default();
                skeleton.set_root(root.id());
                for bone in bones {
                    skeleton.add_bone(bone.path, bone.local, bone.character);
                }
                skeleton
            }
        };

        Ok(skeleton)
    }

    fn extensions(&self) -> &[&str] {
        &["skn.ron"]
    }
}
