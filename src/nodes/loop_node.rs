use crate::core::animation_graph::{
    EdgePath, EdgeSpec, EdgeValue, NodeInput, NodeOutput, TimeState, TimeUpdate,
};
use crate::core::animation_node::{AnimationNode, AnimationNodeType, NodeLike};
use crate::core::graph_context::GraphContext;
use bevy::prelude::*;
use bevy::utils::HashMap;

#[derive(Reflect, Clone, Debug, Default)]
pub struct LoopNode {}

impl LoopNode {
    pub const INPUT: &'static str = "Input Pose";
    pub const OUTPUT: &'static str = "Loop Pose";

    pub fn new() -> Self {
        Self {}
    }

    pub fn wrapped(self, name: impl Into<String>) -> AnimationNode {
        AnimationNode::new_from_nodetype(name.into(), AnimationNodeType::Loop(self))
    }
}

impl NodeLike for LoopNode {
    fn parameter_pass(
        &self,
        _inputs: HashMap<NodeInput, EdgeValue>,
        _name: &str,
        _path: &EdgePath,
        _context: &mut GraphContext,
    ) -> HashMap<NodeOutput, EdgeValue> {
        HashMap::new()
    }

    fn duration_pass(
        &self,
        _inputs: HashMap<NodeInput, Option<f32>>,
        _name: &str,
        _path: &EdgePath,
        _context: &mut GraphContext,
    ) -> Option<f32> {
        None
    }

    fn time_pass(
        &self,
        input: TimeState,
        name: &str,
        _path: &EdgePath,
        context: &mut GraphContext,
    ) -> HashMap<NodeInput, TimeUpdate> {
        let durations = context.get_durations(name).unwrap();
        let duration = *durations.upstream.get(Self::INPUT).unwrap();
        let Some(duration) = duration else {
            return HashMap::from([(Self::INPUT.into(), input.update)]);
        };

        let t = input.time.rem_euclid(duration);

        let fw_upd = match input.update {
            TimeUpdate::Delta(dt) => {
                let prev_time = input.time - dt;
                if prev_time.div_euclid(duration) != input.time.div_euclid(duration) {
                    TimeUpdate::Absolute(t)
                } else {
                    TimeUpdate::Delta(dt)
                }
            }
            TimeUpdate::Absolute(_) => TimeUpdate::Absolute(t),
        };

        HashMap::from([(Self::INPUT.into(), fw_upd)])
    }

    fn time_dependent_pass(
        &self,
        inputs: HashMap<NodeInput, EdgeValue>,
        name: &str,
        path: &EdgePath,
        context: &mut GraphContext,
    ) -> HashMap<NodeOutput, EdgeValue> {
        let time = context.get_times(name, path).unwrap();
        let durations = context.get_durations(name).unwrap();
        let mut in_pose_frame = inputs.get(Self::INPUT).unwrap().clone().unwrap_pose_frame();
        let time = time.downstream.time;
        let duration = durations.upstream.get(Self::INPUT).unwrap();

        if let Some(duration) = duration {
            let t_extra = time.div_euclid(*duration) * duration;
            in_pose_frame.map_ts(|t| t + t_extra);
        }

        HashMap::from([(Self::OUTPUT.into(), EdgeValue::PoseFrame(in_pose_frame))])
    }

    fn parameter_input_spec(&self) -> HashMap<NodeInput, EdgeSpec> {
        HashMap::new()
    }

    fn parameter_output_spec(&self) -> HashMap<NodeOutput, EdgeSpec> {
        HashMap::new()
    }

    fn duration_input_spec(&self) -> HashMap<NodeInput, ()> {
        HashMap::from([(Self::INPUT.into(), ())])
    }

    fn time_dependent_input_spec(&self) -> HashMap<NodeInput, EdgeSpec> {
        HashMap::from([(Self::INPUT.into(), EdgeSpec::PoseFrame)])
    }

    fn time_dependent_output_spec(&self) -> HashMap<NodeOutput, EdgeSpec> {
        HashMap::from([(Self::OUTPUT.into(), EdgeSpec::PoseFrame)])
    }
}
