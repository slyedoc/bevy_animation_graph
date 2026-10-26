//! `.animgraph.bsn`: an animation graph written as entities.
//!
//! The file is a scene: the loader spawns it into a world of its own and builds the graph from
//! those entities ([`graph_from_world`]), which is also how an editor builds it from the live
//! entities it edits. The root entity carries [`AnimGraph`] (the graph's own pins) and [`Links`]
//! (what feeds its outputs). Each child is a node: its `#Name` names it, its node component (any
//! type that reflects `NodeLike`) is what it does, its [`Links`] say what feeds its inputs (a
//! node's output by entity reference) and [`NodePosition`] is where the editor draws it.
//!
//! ```bsn
//! #jump
//! AnimGraph { outputs: [Time, Data("pose", Pose)] }
//! Links([Link(Data("pose"), Node(#clip, "pose")), Link(Time(""), NodeTime(#clip))])
//! Children [
//!     #clip
//!     ClipNode { clip: "anim/ual/Jump_Start.anim.ron" }
//!     NodePosition(40.0, 40.0)
//! ]
//! ```

use bevy::{
    asset::{AssetLoader, AssetPath, AssetServer, Assets, LoadContext, UntypedHandle, io::Reader},
    bsn::BsnDocument,
    bsn_asset::DynamicScene,
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
    reflect::{
        Reflect, ReflectFromReflect, TypePath, TypeRegistry, TypeRegistryArc,
        std_traits::ReflectDefault,
    },
    scene::{ScenePatch, WorldSceneExt},
};
use uuid::Uuid;

use crate::{
    animation_graph::{AnimationGraph, GraphInputPin, NodeId, PinId, SourcePin, TargetPin},
    animation_node::{AnimationNode, ReflectNodeLike, dyn_node_like::DynNodeLike},
    edge_data::{DataSpec, DataValue},
    errors::AssetLoaderError,
};

/// The graph's own pins, in the order the editor lists them.
#[derive(Component, Reflect, Default, Clone, Debug)]
#[reflect(Component, Default, Clone)]
pub struct AnimGraph {
    pub inputs: Vec<GraphInput>,
    pub outputs: Vec<GraphOutput>,
    /// Where the editor draws the graph's inputs box.
    pub input_position: Vec2,
    /// Where the editor draws the graph's outputs box.
    pub output_position: Vec2,
}

#[derive(Reflect, Clone, Debug)]
#[reflect(Default)]
pub enum GraphInput {
    Time(GraphInputPin),
    Data {
        pin: GraphInputPin,
        spec: DataSpec,
        #[reflect(default)]
        default: Option<DataValue>,
    },
}

impl Default for GraphInput {
    fn default() -> Self {
        Self::Time(GraphInputPin::default())
    }
}

#[derive(Reflect, Clone, Debug)]
#[reflect(Default)]
pub enum GraphOutput {
    Time,
    Data(PinId, DataSpec),
}

impl Default for GraphOutput {
    fn default() -> Self {
        Self::Time
    }
}

/// What feeds this entity's inputs: a node's input pins, or on the root, the graph's outputs.
#[derive(Component, FromTemplate, Reflect, Clone, Debug, PartialEq)]
#[template(reflect)]
#[reflect(Component, Clone, FromTemplate)]
pub struct Links(#[template(built_in)] pub Vec<Link>);

/// One input and the output it reads.
#[derive(FromTemplate, Reflect, Clone, Debug, PartialEq)]
#[template(reflect)]
pub struct Link(pub LinkTo, pub LinkFrom);

/// The input end of a [`Link`]. On the root, `Time`'s pin name is ignored: a graph has one
/// time output.
#[derive(Reflect, Clone, Debug, PartialEq)]
#[reflect(Default)]
pub enum LinkTo {
    Data(PinId),
    Time(PinId),
}

impl Default for LinkTo {
    fn default() -> Self {
        Self::Data(PinId::default())
    }
}

/// The output end of a [`Link`]: a node's output (the node entity, `#name` in a file), or one of
/// the graph's inputs.
#[derive(FromTemplate, Reflect, Clone, Debug, PartialEq)]
#[template(reflect)]
pub enum LinkFrom {
    Node(Entity, PinId),
    NodeTime(Entity),
    #[default]
    Input(GraphInputPin),
    InputTime(GraphInputPin),
}

/// Where the editor draws a node.
#[derive(Component, Reflect, Default, Clone, Copy, Debug, PartialEq)]
#[reflect(Component, Default, Clone)]
pub struct NodePosition(pub f32, pub f32);

/// A node's id, derived from its name so the same document always builds the same ids.
pub fn node_id(name: &str) -> NodeId {
    let fnv = |seed: u64| {
        name.bytes()
            .fold(seed, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
    };
    NodeId(Uuid::from_u64_pair(
        fnv(0xcbf29ce484222325),
        fnv(0x84222325cbf29ce4),
    ))
}

/// Builds the graph the entities under `root` describe (a spawned `.animgraph.bsn`, or the
/// entities an editor holds).
pub fn graph_from_world(
    world: &World,
    root: Entity,
    registry: &TypeRegistry,
) -> Result<AnimationGraph, String> {
    let mut graph = AnimationGraph::new();
    let spec = world
        .get::<AnimGraph>(root)
        .ok_or("the root carries no AnimGraph")?;
    for input in &spec.inputs {
        match input {
            GraphInput::Time(pin) => graph.io_spec.add_input_time(pin.clone()),
            GraphInput::Data { pin, spec, default } => {
                graph.io_spec.add_input_data(pin.clone(), *spec);
                if let Some(value) = default {
                    graph.set_default_data(pin.clone(), value.clone());
                }
            }
        }
    }
    for output in &spec.outputs {
        match output {
            GraphOutput::Time => graph.io_spec.add_output_time(),
            GraphOutput::Data(pin, spec) => graph.io_spec.add_output_data(pin.clone(), *spec),
        }
    }
    graph.editor_metadata.input_position = spec.input_position;
    graph.editor_metadata.output_position = spec.output_position;

    let mut edges = Vec::new();
    for link in links(world, root) {
        let target = match &link.0 {
            LinkTo::Data(pin) => TargetPin::OutputData(pin.clone()),
            LinkTo::Time(_) => TargetPin::OutputTime,
        };
        edges.push((source_pin(world, &link.1)?, target));
    }
    for child in world
        .get::<Children>(root)
        .map(|c| c.to_vec())
        .unwrap_or_default()
    {
        let name = world
            .get::<Name>(child)
            .ok_or("a node has no #Name")?
            .as_str()
            .to_string();
        let id = node_id(&name);
        for link in links(world, child) {
            let target = match &link.0 {
                LinkTo::Data(pin) => TargetPin::NodeData(id, pin.clone()),
                LinkTo::Time(pin) => TargetPin::NodeTime(id, pin.clone()),
            };
            edges.push((source_pin(world, &link.1)?, target));
        }
        if let Some(p) = world.get::<NodePosition>(child) {
            graph
                .editor_metadata
                .node_positions
                .insert(id, Vec2::new(p.0, p.1));
        }
        let inner = node_component(world, child, registry)
            .ok_or_else(|| format!("{name}: no node component"))?;
        graph.add_node(AnimationNode {
            id,
            name,
            inner,
            should_debug: false,
        });
    }
    // A link from a node that is gone (deleted in an editor, say) is dropped, not fatal.
    for (source, target) in edges {
        let from = match &source {
            SourcePin::NodeData(id, _) | SourcePin::NodeTime(id) => Some(*id),
            _ => None,
        };
        if from.is_some_and(|id| !graph.nodes.contains_key(&id)) {
            bevy::log::warn!("dropping a link from a node the graph does not have: {source:?}");
            continue;
        }
        graph.add_edge(source, target);
    }
    graph.validate().map_err(|e| e.to_string())?;
    Ok(graph)
}

fn links(world: &World, entity: Entity) -> Vec<Link> {
    world
        .get::<Links>(entity)
        .map(|l| l.0.clone())
        .unwrap_or_default()
}

/// The entity's node component (the one reflecting `NodeLike`), cloned.
fn node_component(world: &World, entity: Entity, registry: &TypeRegistry) -> Option<DynNodeLike> {
    let entity_ref = world.get_entity(entity).ok()?;
    entity_ref.archetype().components().iter().find_map(|&id| {
        let type_id = world.components().get_info(id)?.type_id()?;
        let registration = registry.get(type_id)?;
        let node_like = registration.data::<ReflectNodeLike>()?;
        let value = registration
            .data::<ReflectComponent>()?
            .reflect(entity_ref)?;
        let owned = value.reflect_clone().ok().or_else(|| {
            registration
                .data::<ReflectFromReflect>()?
                .from_reflect(value.as_partial_reflect())
        })?;
        node_like.get_boxed(owned).ok().map(DynNodeLike)
    })
}

/// The id of the node an entity is: its name's.
fn node_of(world: &World, entity: Entity) -> Result<NodeId, String> {
    world
        .get::<Name>(entity)
        .map(|name| node_id(name.as_str()))
        .ok_or_else(|| "a link reads from an entity that is not a named node".to_string())
}

fn source_pin(world: &World, from: &LinkFrom) -> Result<SourcePin, String> {
    Ok(match from {
        LinkFrom::Node(entity, pin) => SourcePin::NodeData(node_of(world, *entity)?, pin.clone()),
        LinkFrom::NodeTime(entity) => SourcePin::NodeTime(node_of(world, *entity)?),
        LinkFrom::Input(pin) => SourcePin::InputData(pin.clone()),
        LinkFrom::InputTime(pin) => SourcePin::InputTime(pin.clone()),
    })
}

/// Parses `text` and spawns it, as a scene, into a world of its own; handles load through
/// `handles` (the load context, in a loader).
pub fn spawn_document(
    text: &str,
    path: &str,
    registry: &TypeRegistryArc,
    server: &AssetServer,
    handles: &mut dyn FnMut(core::any::TypeId, AssetPath<'static>) -> UntypedHandle,
) -> Result<(World, Entity), String> {
    let document = BsnDocument::parse(text).map_err(|e| format!("{e:?}"))?;
    let registry = AppTypeRegistry(registry.clone());
    let scene = DynamicScene::from_document_with_handles(&document, path, &registry, handles)
        .map_err(|e| e.render(text))?;
    let mut world = World::new();
    world.insert_resource(registry);
    world.insert_resource(server.clone());
    world.init_resource::<Assets<ScenePatch>>();
    let root = world.spawn_scene(scene).map_err(|e| format!("{e:?}"))?.id();
    Ok((world, root))
}

/// Loads `.animgraph.bsn` files as [`AnimationGraph`]s.
#[derive(TypePath)]
pub struct AnimGraphBsnLoader {
    registry: TypeRegistryArc,
    server: AssetServer,
}

impl FromWorld for AnimGraphBsnLoader {
    fn from_world(world: &mut World) -> Self {
        Self {
            registry: world.resource::<AppTypeRegistry>().0.clone(),
            server: world.resource::<AssetServer>().clone(),
        }
    }
}

impl AssetLoader for AnimGraphBsnLoader {
    type Asset = AnimationGraph;
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
        graph_from_world(&world, root, &self.registry.read())
            .map_err(|e| AssetLoaderError::Bsn(format!("{path}: {e}")))
    }

    fn extensions(&self) -> &[&str] {
        &["animgraph.bsn"]
    }
}
