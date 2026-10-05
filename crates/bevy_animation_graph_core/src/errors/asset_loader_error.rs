use bevy::prelude::*;
use thiserror::Error;

use super::GraphValidationError;

/// Possible errors that can be produced by a custom asset loader
// TODO: clean this up
// https://rust-lang.github.io/api-guidelines/interoperability.html?highlight=error#examples-of-error-messages
// - lowercase error messages
// - don't print sources exclusively
// - avoid mega error enums
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum AssetLoaderError {
    /// An [IO](std::io) Error
    #[error("Could load shader: {0}")]
    Io(#[from] std::io::Error),
    /// A [RON](ron) Error
    #[error("Could not parse RON: {0}")]
    RonSpannedError(#[from] ron::error::SpannedError),
    #[error("Could not read a baked .animclip: {0}")]
    AnimClipError(#[from] crate::animation_clip::animclip::AnimClipError),
    #[error("Could not read a referenced asset: {0}")]
    ReadAssetBytesError(#[from] bevy::asset::ReadAssetBytesError),
    #[error("Animated scene path is incorrect: {0}")]
    AnimatedSceneMissingName(String),
    #[error(
        "Animated scene missing a root (an exsiting AnimationPlayer). A possible cause is that your source scene does not have any animations."
    )]
    AnimatedSceneMissingRoot,
    #[error("Graph does not satisfy constraints: {0}")]
    InconsistentGraphError(#[from] GraphValidationError),
    #[error("Failed to load skeleton colliders object")]
    SkeletonColliderLoadError,
    #[error("Failed to parse a provided regular expression: {0}")]
    RegexParsingError(#[from] regex::Error),
    #[error("Could not read a .bsn graph: {0}")]
    Bsn(String),
}
