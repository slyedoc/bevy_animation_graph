use crate::{
    core::{
        animation_graph::{PinMap, TimeUpdate},
        animation_node::{AnimationNode, AnimationNodeType, NodeLike},
        duration_data::DurationData,
        errors::GraphError,
        frame::{BonePoseFrame, PoseFrame, PoseFrameData, PoseSpec},
        space_conversion::SpaceConversion,
    },
    prelude::{PassContext, SpecContext},
    utils::unwrap::Unwrap,
};
use bevy::reflect::{std_traits::ReflectDefault, Reflect};

#[derive(Reflect, Clone, Debug, Default)]
#[reflect(Default)]
pub struct ExtendSkeleton {}

impl ExtendSkeleton {
    pub const POSE_IN: &'static str = "Pose In";

    pub fn new() -> Self {
        Self {}
    }

    pub fn wrapped(self, name: impl Into<String>) -> AnimationNode {
        AnimationNode::new_from_nodetype(name.into(), AnimationNodeType::ExtendSkeleton(self))
    }
}

impl NodeLike for ExtendSkeleton {
    fn duration_pass(&self, mut ctx: PassContext) -> Result<Option<DurationData>, GraphError> {
        Ok(Some(ctx.duration_back(Self::POSE_IN)?))
    }

    fn pose_pass(
        &self,
        time_update: TimeUpdate,
        mut ctx: PassContext,
    ) -> Result<Option<PoseFrame>, GraphError> {
        let in_pose = ctx.pose_back(Self::POSE_IN, time_update)?;
        let bone_pose_frame: BonePoseFrame = in_pose.data.unwrap();

        Ok(Some(PoseFrame {
            data: PoseFrameData::BoneSpace(ctx.extend_skeleton_bone(&bone_pose_frame)),
            ..in_pose
        }))
    }

    fn pose_input_spec(&self, _ctx: SpecContext) -> PinMap<PoseSpec> {
        [(Self::POSE_IN.into(), PoseSpec::BoneSpace)].into()
    }

    fn pose_output_spec(&self, _ctx: SpecContext) -> Option<PoseSpec> {
        Some(PoseSpec::BoneSpace)
    }

    fn display_name(&self) -> String {
        "Extend Skeleton".into()
    }
}
