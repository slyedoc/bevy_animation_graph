use bevy::{ecs::entity::Entity, utils::HashMap};

use crate::{
    core::{
        animation_graph::{InputOverlay, NodeId, PinId, SourcePin, TargetPin, TimeUpdate},
        duration_data::DurationData,
        errors::GraphError,
        frame::PoseFrame,
        pose::BoneId,
    },
    prelude::{AnimationGraph, ParamValue},
};

use super::{deferred_gizmos::DeferredGizmoRef, GraphContext, SystemResources};

#[derive(Clone, Copy)]
pub struct NodeContext<'a> {
    pub node_id: &'a NodeId,
    pub graph: &'a AnimationGraph,
}

#[derive(Clone)]
pub struct PassContext<'a> {
    pub context: GraphContextRef,
    pub resources: &'a SystemResources<'a, 'a>,
    pub overlay: &'a InputOverlay,
    pub node_context: Option<NodeContext<'a>>,
    pub parent: Option<PassContextRef<'a>>,
    pub root_entity: Entity,
    pub entity_map: &'a HashMap<BoneId, Entity>,
    pub deferred_gizmos: DeferredGizmoRef,
    pub should_debug: bool,
}

impl<'a> PassContext<'a> {
    /// Creates a pass context with no parent graph nor node context data
    pub fn new(
        context: &mut GraphContext,
        resources: &'a SystemResources,
        overlay: &'a InputOverlay,
        root_entity: Entity,
        entity_map: &'a HashMap<BoneId, Entity>,
        deferred_gizmos: impl Into<DeferredGizmoRef>,
    ) -> Self {
        Self {
            context: context.into(),
            resources,
            overlay,
            node_context: None,
            parent: None,
            root_entity,
            entity_map,
            deferred_gizmos: deferred_gizmos.into(),
            should_debug: false,
        }
    }

    /// Decorates a pass context with node data. Usually done by `AnimationGraph` before
    /// passing the context down to a node.
    pub fn with_node(&self, node_id: &'a NodeId, graph: &'a AnimationGraph) -> Self {
        Self {
            context: self.context.clone(),
            resources: self.resources,
            overlay: self.overlay,
            node_context: Some(NodeContext { node_id, graph }),
            parent: self.parent.clone(),
            root_entity: self.root_entity,
            entity_map: self.entity_map,
            deferred_gizmos: self.deferred_gizmos.clone(),
            should_debug: self.should_debug,
        }
    }

    /// Returns a pass context with node data cleared. Usually done before passing the
    /// context back up to the graph to request further inputs.
    pub fn without_node(&self) -> Self {
        Self {
            context: self.context.clone(),
            resources: self.resources,
            overlay: self.overlay,
            node_context: None,
            parent: self.parent.clone(),
            root_entity: self.root_entity,
            entity_map: self.entity_map,
            deferred_gizmos: self.deferred_gizmos.clone(),
            should_debug: self.should_debug,
        }
    }

    /// Returns a pass context with updated `should_debug`
    pub fn with_debugging(&self, should_debug: bool) -> Self {
        Self {
            context: self.context.clone(),
            resources: self.resources,
            overlay: self.overlay,
            node_context: self.node_context,
            parent: self.parent.clone(),
            root_entity: self.root_entity,
            entity_map: self.entity_map,
            deferred_gizmos: self.deferred_gizmos.clone(),
            should_debug,
        }
    }

    /// Returns a new pass context decorated with `self` as the parent context.
    /// Used when passing the context down to a subgraph.
    pub fn child(&'a self, overlay: &'a InputOverlay) -> Self {
        Self {
            context: self
                .context
                .as_mut()
                .context_for_subgraph_or_insert_default(self.node_context.unwrap().node_id),
            resources: self.resources,
            overlay,
            node_context: self.node_context,
            parent: Some(self.into()),
            root_entity: self.root_entity,
            entity_map: self.entity_map,
            deferred_gizmos: self.deferred_gizmos.clone(),
            should_debug: self.should_debug,
        }
    }

    /// Access the parent pass context.
    pub fn parent(&'a self) -> Self {
        self.parent.as_ref().unwrap().as_ref()
    }

    /// Verify whether the current context has a parent. Should be true when inside a subgraph and
    /// false otherwise
    pub fn has_parent(&self) -> bool {
        self.parent.is_some()
    }

    /// Return a mutable reference to the `GraphContext`
    pub fn context(&mut self) -> &mut GraphContext {
        self.context.as_mut()
    }

    /// Request an input parameter from the graph
    pub fn parameter_back(&mut self, pin_id: impl Into<PinId>) -> Result<ParamValue, GraphError> {
        let node_ctx = self.node_context.unwrap();
        let target_pin = TargetPin::NodeParameter(node_ctx.node_id.clone(), pin_id.into());
        node_ctx
            .graph
            .get_parameter(target_pin, self.without_node())
    }

    /// Request the duration of an input pose pin.
    pub fn duration_back(&mut self, pin_id: impl Into<PinId>) -> Result<DurationData, GraphError> {
        let node_ctx = self.node_context.unwrap();
        let target_pin = TargetPin::NodePose(node_ctx.node_id.clone(), pin_id.into());
        node_ctx.graph.get_duration(target_pin, self.without_node())
    }

    /// Request an input pose.
    pub fn pose_back(
        &mut self,
        pin_id: impl Into<PinId>,
        time_update: TimeUpdate,
    ) -> Result<PoseFrame, GraphError> {
        let node_ctx = self.node_context.unwrap();
        let target_pin = TargetPin::NodePose(node_ctx.node_id.clone(), pin_id.into());
        node_ctx
            .graph
            .get_pose(time_update, target_pin, self.without_node())
    }

    /// Request the cached time update query from the current frame
    pub fn time_update_fwd(&self) -> TimeUpdate {
        let node_ctx = self.node_context.unwrap();
        let source_pin = SourcePin::NodePose(node_ctx.node_id.clone());

        *self
            .context
            .as_mut()
            .get_time_update(&source_pin)
            .unwrap_or_else(|| panic!("Time update not cached at {source_pin:?}"))
    }

    /// Request the cached timestamp of the output animation in the last frame
    pub fn prev_time_fwd(&self) -> f32 {
        let node_ctx = self.node_context.unwrap();
        let source_pin = SourcePin::NodePose(node_ctx.node_id.clone());
        self.context.as_mut().get_prev_time(&source_pin)
    }
}

#[derive(Clone)]
pub struct PassContextRef<'a> {
    ctx: *const PassContext<'a>,
}

impl<'a> From<&'a PassContext<'a>> for PassContextRef<'a> {
    fn from(value: &'a PassContext) -> Self {
        Self { ctx: value }
    }
}

impl<'a> PassContextRef<'a> {
    pub fn as_ref(&self) -> PassContext<'a> {
        unsafe { self.ctx.as_ref().unwrap().clone() }
    }
}

// Internal mutability lets goooooooooo
// May god have mercy on us
#[derive(Clone)]
pub struct GraphContextRef {
    context: *mut GraphContext,
}

impl From<&mut GraphContext> for GraphContextRef {
    fn from(value: &mut GraphContext) -> Self {
        Self { context: value }
    }
}

impl GraphContextRef {
    #[allow(clippy::mut_from_ref)]
    pub fn as_mut(&self) -> &mut GraphContext {
        unsafe { self.context.as_mut().unwrap() }
    }
}
