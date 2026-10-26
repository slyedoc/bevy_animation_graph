//! `.fsm.bsn`: a state machine written as entities.
//!
//! A scene, like `.animgraph.bsn`: the loader spawns it and builds the machine from the entities
//! ([`fsm_from_world`]). The root carries [`AnimFsm`] (start state and pins). Each child carrying
//! [`AnimState`] is a state, its `#Name` the state's label; each carrying [`AnimTransition`] is a
//! direct transition between two states. States are referenced by entity (`#name` in a file).
//!
//! ```bsn
//! #locomotion
//! AnimFsm { start: #grounded, inputs: [Data("speed", F32)], outputs: [Time, Data("pose", Pose)] }
//! Children [
//!     #grounded
//!     AnimState { graph: "anim/ual/grounded.animgraph.bsn" }
//!     NodePosition(0.0, 0.0)
//! ]
//! ```

use bevy::{
    asset::{AssetLoader, AssetServer, Handle, LoadContext, io::Reader},
    ecs::{
        component::Component,
        entity::Entity,
        hierarchy::Children,
        name::Name,
        reflect::{AppTypeRegistry, ReflectComponent, ReflectFromTemplate},
        template::FromTemplate,
        world::{FromWorld, World},
    },
    math::Vec2,
    reflect::{Reflect, TypePath, TypeRegistryArc, std_traits::ReflectDefault},
};
use uuid::Uuid;

use crate::{
    animation_graph::{
        AnimationGraph, PinId,
        bsn::{GraphOutput, NodePosition, spawn_document},
    },
    context::spec_context::NodeSpec,
    edge_data::DataSpec,
    errors::AssetLoaderError,
    state_machine::high_level::{
        DirectTransition, DirectTransitionId, State, StateId, StateMachine, TransitionData,
    },
};

/// The state machine's start state and pins.
#[derive(Component, FromTemplate, Reflect, Clone, Debug)]
#[template(reflect)]
#[reflect(Component, Clone, FromTemplate)]
pub struct AnimFsm {
    pub start: Entity,
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

/// A direct transition between two states.
#[derive(Component, FromTemplate, Reflect, Clone, Debug)]
#[template(reflect)]
#[reflect(Component, Clone, FromTemplate)]
pub struct AnimTransition {
    pub from: Entity,
    pub to: Entity,
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

/// Builds the state machine the entities under `root` describe (a spawned `.fsm.bsn`, or the
/// entities an editor holds).
pub fn fsm_from_world(world: &World, root: Entity) -> Result<StateMachine, String> {
    let spec = world
        .get::<AnimFsm>(root)
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
    let label = |entity: Entity| world.get::<Name>(entity).map(|n| n.as_str().to_string());
    let mut labels = Vec::new();
    for child in world
        .get::<Children>(root)
        .map(|c| c.to_vec())
        .unwrap_or_default()
    {
        let name = label(child).unwrap_or_default();
        if let Some(state) = world.get::<AnimState>(child) {
            fsm.add_state(State {
                id: state_id(&name),
                label: name.clone(),
                graph: state.graph.clone(),
                state_transition: state.enter.clone(),
            });
            labels.push(name.clone());
        }
        if let Some(t) = world.get::<AnimTransition>(child) {
            let (Some(from), Some(to)) = (label(t.from), label(t.to)) else {
                bevy::log::warn!("dropping a transition between entities that are not states");
                continue;
            };
            fsm.add_transition_unchecked(DirectTransition {
                id: DirectTransitionId(id_of(&format!("{from}->{to}"))),
                source: state_id(&from),
                target: state_id(&to),
                data: t.data.clone(),
            });
        }
        if let Some(p) = world.get::<NodePosition>(child) {
            fsm.editor_metadata
                .set_state_position(state_id(&name), Vec2::new(p.0, p.1));
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
    let start = label(spec.start)
        .filter(|start| labels.contains(start))
        .ok_or("the start state is not a state")?;
    fsm.set_start_state(state_id(&start));
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
        let mut handles =
            |type_id, asset_path| load_context.load_builder().load_erased(type_id, asset_path);
        let (world, root) = spawn_document(text, &path, &self.registry, &self.server, &mut handles)
            .map_err(|e| AssetLoaderError::Bsn(format!("{path}: {e}")))?;
        fsm_from_world(&world, root).map_err(|e| AssetLoaderError::Bsn(format!("{path}: {e}")))
    }

    fn extensions(&self) -> &[&str] {
        &["fsm.bsn"]
    }
}
