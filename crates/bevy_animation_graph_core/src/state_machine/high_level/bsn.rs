//! `.fsm.bsn`: a state machine written as entities.
//!
//! The root carries [`AnimFsm`] (start state and pins). Each child carrying [`AnimState`] is a state,
//! its `#Name` the state's label; each carrying [`AnimTransition`] is a direct transition between
//! two states, by label.
//!
//! ```bsn
//! #locomotion
//! AnimFsm { start: "grounded", inputs: [Data("speed", F32)], outputs: [Time, Data("pose", Pose)] }
//! Children [
//!     #grounded
//!     AnimState { graph: "anim/ual/grounded.animgraph.bsn" }
//!     NodePosition(0.0, 0.0)
//! ]
//! ```

use bevy::{
    asset::{AssetLoader, AssetServer, Handle, LoadContext, io::Reader},
    bsn_document::{BsnApplyAssets, SceneBsnAst, parse_bsn},
    ecs::{
        component::Component,
        entity::Entity,
        reflect::{AppTypeRegistry, ReflectComponent},
        world::{FromWorld, World},
    },
    math::Vec2,
    reflect::{Reflect, TypePath, TypeRegistry, TypeRegistryArc, std_traits::ReflectDefault},
};
use uuid::Uuid;

use crate::{
    animation_graph::{
        AnimationGraph, PinId,
        bsn::{GraphOutput, NodePosition, entity_values},
    },
    context::spec_context::NodeSpec,
    edge_data::DataSpec,
    errors::AssetLoaderError,
    state_machine::high_level::{
        DirectTransition, DirectTransitionId, State, StateId, StateMachine, TransitionData,
    },
};

/// The state machine's start state (by label) and pins.
#[derive(Component, Reflect, Default, Clone, Debug)]
#[reflect(Component, Default, Clone)]
pub struct AnimFsm {
    pub start: String,
    pub inputs: Vec<FsmInput>,
    pub outputs: Vec<GraphOutput>,
}

#[derive(Reflect, Clone, Debug)]
#[reflect(Default)]
pub enum FsmInput {
    Time(PinId),
    Data(PinId, DataSpec),
}

impl Default for FsmInput {
    fn default() -> Self {
        Self::Time(PinId::default())
    }
}

/// A state: the graph it plays and, optionally, how any state enters it.
#[derive(Component, Reflect, Default, Clone, Debug)]
#[reflect(Component, Default, Clone)]
pub struct AnimState {
    pub graph: Handle<AnimationGraph>,
    pub enter: Option<TransitionData>,
}

/// A direct transition between two states, by label.
#[derive(Component, Reflect, Default, Clone, Debug)]
#[reflect(Component, Default, Clone)]
pub struct AnimTransition {
    pub from: String,
    pub to: String,
    pub data: TransitionData,
}

fn id_of(name: &str) -> Uuid {
    let fnv = |seed: u64| {
        name.bytes()
            .fold(seed, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
    };
    Uuid::from_u64_pair(fnv(0xcbf29ce484222325), fnv(0x84222325cbf29ce4))
}

/// A state's id, derived from its label.
pub fn state_id(label: &str) -> StateId {
    StateId(id_of(label))
}

/// Builds the state machine a parsed `.fsm.bsn` describes, from its root entity.
pub fn fsm_from_document(
    ast: &SceneBsnAst,
    root: Entity,
    registry: &TypeRegistry,
    assets: Option<&BsnApplyAssets>,
) -> Result<StateMachine, String> {
    let spec = entity_values(ast, root, registry, assets)?
        .into_iter()
        .find_map(|v| v.downcast_ref::<AnimFsm>().cloned())
        .ok_or("the root carries no AnimFsm")?;
    let mut node_spec = NodeSpec::default();
    for input in &spec.inputs {
        match input {
            FsmInput::Time(pin) => node_spec.add_input_time(pin.clone()),
            FsmInput::Data(pin, data) => node_spec.add_input_data(pin.clone(), *data),
        }
    }
    for output in &spec.outputs {
        match output {
            GraphOutput::Time => node_spec.add_output_time(),
            GraphOutput::Data(pin, data) => node_spec.add_output_data(pin.clone(), *data),
        }
    }
    let mut fsm = StateMachine {
        node_spec,
        ..Default::default()
    };
    let mut labels = Vec::new();
    for child in ast.get_children_ast(root) {
        let name = ast.get_name(child).unwrap_or_default().to_string();
        let mut position = None;
        for value in entity_values(ast, child, registry, assets)? {
            if let Some(state) = value.downcast_ref::<AnimState>() {
                fsm.add_state(State {
                    id: state_id(&name),
                    label: name.clone(),
                    graph: state.graph.clone(),
                    state_transition: state.enter.clone(),
                });
                labels.push(name.clone());
            } else if let Some(t) = value.downcast_ref::<AnimTransition>() {
                fsm.add_transition_unchecked(DirectTransition {
                    id: DirectTransitionId(id_of(&format!("{}->{}", t.from, t.to))),
                    source: state_id(&t.from),
                    target: state_id(&t.to),
                    data: t.data.clone(),
                });
            } else if let Some(p) = value.downcast_ref::<NodePosition>() {
                position = Some(Vec2::new(p.0, p.1));
            }
        }
        if let Some(position) = position {
            fsm.editor_metadata
                .set_state_position(state_id(&name), position);
        }
    }
    // A transition to or from a state that is gone is dropped, not fatal.
    let states = fsm.states.clone();
    fsm.transitions.retain(|_, t| {
        let known = states.contains_key(&t.source) && states.contains_key(&t.target);
        if !known {
            bevy::log::warn!("dropping a transition between states the machine does not have");
        }
        known
    });
    if !labels.contains(&spec.start) {
        return Err(format!("start state `{}` is not a state", spec.start));
    }
    fsm.set_start_state(state_id(&spec.start));
    fsm.update_low_level_fsm();
    Ok(fsm)
}

/// Loads `.fsm.bsn` files as [`StateMachine`]s.
#[derive(TypePath)]
pub struct FsmBsnLoader {
    registry: TypeRegistryArc,
    server: AssetServer,
}

impl FromWorld for FsmBsnLoader {
    fn from_world(world: &mut World) -> Self {
        Self {
            registry: world.resource::<AppTypeRegistry>().0.clone(),
            server: world.resource::<AssetServer>().clone(),
        }
    }
}

impl AssetLoader for FsmBsnLoader {
    type Asset = StateMachine;
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
        let path = load_context.path().to_string();
        let text = std::str::from_utf8(&bytes)
            .map_err(|e| AssetLoaderError::Bsn(format!("{path}: {e}")))?;
        let ast = parse_bsn(text).map_err(|e| AssetLoaderError::Bsn(format!("{path}: {e:?}")))?;
        let [root] = ast.roots[..] else {
            return Err(AssetLoaderError::Bsn(format!(
                "{path}: a state machine document has one root entity, not {}",
                ast.roots.len()
            )));
        };
        let assets = BsnApplyAssets {
            server: &self.server,
            local: None,
        };
        fsm_from_document(&ast, root, &self.registry.read(), Some(&assets))
            .map_err(|e| AssetLoaderError::Bsn(format!("{path}: {e}")))
    }

    fn extensions(&self) -> &[&str] {
        &["fsm.bsn"]
    }
}
