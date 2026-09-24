use bevy::{
    asset::{AssetLoader, AssetPath, LoadContext, io::Reader},
    platform::collections::HashMap,
    reflect::{Reflect, TypePath},
};
use serde::{Deserialize, Serialize};

use super::{GraphClip, animclip::parse_animclip};
use crate::{errors::AssetLoaderError, event_track::EventTrack, utils::normalize_asset_path};

/// Where a [`GraphClip`]'s curves come from.
///
/// On the aurora branch that is always a baked `.animclip`: the importer
/// (`aurora_files`' `animlib_import`) owns glTF, retargets the source library onto the rig at
/// bake time and writes the clip beside the `.bsn`. Nothing loads glTF at runtime, and this
/// bevy no longer offers the immediate nested load the glTF source needed.
#[derive(Reflect, Serialize, Deserialize, Clone, Debug)]
pub enum GraphClipSource {
    AnimClip { path: AssetPath<'static> },
}

#[derive(Serialize, Deserialize, Clone)]
pub struct GraphClipSerial {
    pub source: GraphClipSource,
    pub skeleton: AssetPath<'static>,
    #[serde(default)]
    pub event_tracks: HashMap<String, EventTrack>,
}

#[derive(Default, TypePath)]
pub struct GraphClipLoader;

impl AssetLoader for GraphClipLoader {
    type Asset = GraphClip;
    type Settings = ();
    type Error = AssetLoaderError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = vec![];
        reader.read_to_end(&mut bytes).await?;
        let serial: GraphClipSerial = ron::de::from_bytes(&bytes)?;

        let bevy_clip = match &serial.source {
            GraphClipSource::AnimClip { path } => {
                let clip_bytes = load_context.read_asset_bytes(path.clone()).await?;
                parse_animclip(&clip_bytes)?
            }
        };

        let skeleton = load_context.load(serial.skeleton.clone());

        Ok(GraphClip::from_bevy_clip(
            bevy_clip,
            skeleton,
            serial.event_tracks,
            Some(serial.source.clone()),
        ))
    }

    fn extensions(&self) -> &[&str] {
        &["anim.ron"]
    }
}

impl TryFrom<&GraphClip> for GraphClipSerial {
    type Error = ();

    fn try_from(value: &GraphClip) -> Result<Self, Self::Error> {
        let Some(source) = value.source.clone() else {
            return Err(());
        };

        Ok(Self {
            source,
            skeleton: normalize_asset_path(value.skeleton.path().cloned().ok_or(())?),
            event_tracks: value.event_tracks.clone(),
        })
    }
}
