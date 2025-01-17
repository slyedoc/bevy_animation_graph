use crate::core::animation_graph::{PinMap, TimeUpdate};
use crate::core::animation_node::{AnimationNode, AnimationNodeType, NodeLike};
use crate::core::duration_data::DurationData;
use crate::core::errors::GraphError;
use crate::core::frame::{PoseFrame, PoseSpec};
use crate::interpolation::linear::InterpolateLinear;
use crate::prelude::{OptParamSpec, ParamSpec, PassContext, SpecContext};
use bevy::prelude::*;

#[derive(Reflect, Clone, Debug, Default)]
#[reflect(Default)]
pub struct BlendNode;

impl BlendNode {
    pub const INPUT_1: &'static str = "Pose In 1";
    pub const INPUT_2: &'static str = "Pose In 2";
    pub const FACTOR: &'static str = "Factor";
    pub const OUTPUT: &'static str = "Pose Out";

    pub fn new() -> Self {
        Self
    }

    pub fn wrapped(self, name: impl Into<String>) -> AnimationNode {
        AnimationNode::new_from_nodetype(name.into(), AnimationNodeType::Blend(self))
    }
}

impl NodeLike for BlendNode {
    fn duration_pass(&self, mut ctx: PassContext) -> Result<Option<DurationData>, GraphError> {
        let duration_1 = ctx.duration_back(Self::INPUT_1)?;
        let duration_2 = ctx.duration_back(Self::INPUT_2)?;

        let out_duration = match (duration_1, duration_2) {
            (Some(duration_1), Some(duration_2)) => Some(duration_1.max(duration_2)),
            (Some(duration_1), None) => Some(duration_1),
            (None, Some(duration_2)) => Some(duration_2),
            (None, None) => None,
        };

        Ok(Some(out_duration))
    }

    fn pose_pass(
        &self,
        input: TimeUpdate,
        mut ctx: PassContext,
    ) -> Result<Option<PoseFrame>, GraphError> {
        let in_frame_1 = ctx.pose_back(Self::INPUT_1, input)?;
        let in_frame_2 = ctx.pose_back(Self::INPUT_2, input)?;

        let alpha = ctx.parameter_back(Self::FACTOR)?.unwrap_f32();
        let out = in_frame_1.interpolate_linear(&in_frame_2, alpha);

        Ok(Some(out))
    }

    fn parameter_input_spec(&self, _: SpecContext) -> PinMap<OptParamSpec> {
        [(Self::FACTOR.into(), ParamSpec::F32.into())].into()
    }

    fn pose_input_spec(&self, _: SpecContext) -> PinMap<PoseSpec> {
        [
            (Self::INPUT_1.into(), PoseSpec::Any),
            (Self::INPUT_2.into(), PoseSpec::Any),
        ]
        .into()
    }

    fn pose_output_spec(&self, _: SpecContext) -> Option<PoseSpec> {
        Some(PoseSpec::BoneSpace)
    }

    fn display_name(&self) -> String {
        "∑ Blend".into()
    }
}
