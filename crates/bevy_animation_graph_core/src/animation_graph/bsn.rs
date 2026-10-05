//! `.animgraph.bsn`: an animation graph written as entities.
//!
//! The root entity carries [`AnimGraph`] (the graph's own pins) and [`Links`] (what feeds its
//! outputs). Each child is a node: its `#Name` names it, its node component (any type that
//! reflects `NodeLike`) is what it does, its [`Links`] say what feeds its inputs and
//! [`NodePosition`] is where the editor draws it.
//!
//! ```bsn
//! #jump
//! AnimGraph { outputs: [Time, Data("pose", Pose)] }
//! Links([Link(Data("pose"), Node("clip", "pose")), Link(Time(""), NodeTime("clip"))])
//! Children [
//!     #clip
//!     ClipNode { clip: "anim/ual/Jump_Start.anim.ron" }
//!     NodePosition(40.0, 40.0)
//! ]
//! ```

use bevy::{
    asset::{AssetLoader, AssetServer, LoadContext, io::Reader},
    bsn_document::{
        BsnApplyAssets, BsnPatch, BsnValue, SceneBsnAst, bsn_value_to_reflect, parse_bsn,
    },
    ecs::{
        component::Component,
        entity::Entity,
        reflect::{AppTypeRegistry, ReflectComponent},
        world::{FromWorld, World},
    },
    math::Vec2,
    reflect::{
        PartialReflect, Reflect, ReflectFromReflect, TypePath, TypeRegistration, TypeRegistry,
        TypeRegistryArc, std_traits::ReflectDefault,
    },
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
#[derive(Component, Reflect, Default, Clone, Debug)]
#[reflect(Component, Default, Clone)]
pub struct Links(pub Vec<Link>);

/// One input and the output it reads.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
#[reflect(Default)]
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

/// The output end of a [`Link`]: a node's output by node name, or one of the graph's inputs.
#[derive(Reflect, Clone, Debug, PartialEq)]
#[reflect(Default)]
pub enum LinkFrom {
    Node(String, PinId),
    NodeTime(String),
    Input(GraphInputPin),
    InputTime(GraphInputPin),
}

impl Default for LinkFrom {
    fn default() -> Self {
        Self::NodeTime(String::default())
    }
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

/// Builds the graph a parsed `.animgraph.bsn` describes, from its root entity.
///
/// Handle fields resolve through `assets`; without it they stay default handles.
pub fn graph_from_document(
    ast: &SceneBsnAst,
    root: Entity,
    registry: &TypeRegistry,
    assets: Option<&BsnApplyAssets>,
) -> Result<AnimationGraph, String> {
    let mut graph = AnimationGraph::new();
    let mut spec = None;
    let mut root_links = Vec::new();
    for value in entity_values(ast, root, registry, assets)? {
        if let Some(g) = value.downcast_ref::<AnimGraph>() {
            spec = Some(g.clone());
        } else if let Some(l) = value.downcast_ref::<Links>() {
            root_links = l.0.clone();
        }
    }
    let spec = spec.ok_or("the root carries no AnimGraph")?;
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
    for link in root_links {
        let target = match &link.0 {
            LinkTo::Data(pin) => TargetPin::OutputData(pin.clone()),
            LinkTo::Time(_) => TargetPin::OutputTime,
        };
        edges.push((source_pin(&link.1), target));
    }
    for child in ast.get_children_ast(root) {
        let name = ast
            .get_name(child)
            .ok_or("a node has no #Name")?
            .to_string();
        let id = node_id(&name);
        let mut inner = None;
        for value in entity_values(ast, child, registry, assets)? {
            if let Some(l) = value.downcast_ref::<Links>() {
                for link in &l.0 {
                    let target = match &link.0 {
                        LinkTo::Data(pin) => TargetPin::NodeData(id, pin.clone()),
                        LinkTo::Time(pin) => TargetPin::NodeTime(id, pin.clone()),
                    };
                    edges.push((source_pin(&link.1), target));
                }
            } else if let Some(p) = value.downcast_ref::<NodePosition>() {
                graph
                    .editor_metadata
                    .node_positions
                    .insert(id, Vec2::new(p.0, p.1));
            } else if let Some(node_like) = registry
                .get((*value).type_id())
                .and_then(|r| r.data::<ReflectNodeLike>())
            {
                let boxed = node_like
                    .get_boxed(value)
                    .map_err(|_| format!("{name}: not a NodeLike"))?;
                inner = Some(DynNodeLike(boxed));
            }
        }
        let inner = inner.ok_or_else(|| format!("{name}: no node component"))?;
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

fn source_pin(from: &LinkFrom) -> SourcePin {
    match from {
        LinkFrom::Node(node, pin) => SourcePin::NodeData(node_id(node), pin.clone()),
        LinkFrom::NodeTime(node) => SourcePin::NodeTime(node_id(node)),
        LinkFrom::Input(pin) => SourcePin::InputData(pin.clone()),
        LinkFrom::InputTime(pin) => SourcePin::InputTime(pin.clone()),
    }
}

/// Every component patch on one document entity, reflected into its concrete type. Types the
/// registry does not know are skipped.
pub(crate) fn entity_values(
    ast: &SceneBsnAst,
    entity: Entity,
    registry: &TypeRegistry,
    assets: Option<&BsnApplyAssets>,
) -> Result<Vec<Box<dyn Reflect>>, String> {
    let Some(patches) = ast.get_patches(entity) else {
        return Ok(Vec::new());
    };
    let mut values = Vec::new();
    for &patch in &patches.0 {
        let (type_path, value) = match ast.get_patch(patch) {
            Some(BsnPatch::Struct(data)) => (&data.type_path, BsnValue::Struct(data.clone())),
            Some(BsnPatch::TupleStruct(data)) => {
                (&data.type_path, BsnValue::TupleStruct(data.clone()))
            }
            Some(BsnPatch::Type(type_path)) => (type_path, BsnValue::Type(type_path.clone())),
            _ => continue,
        };
        let Some(registration) = registration_of(registry, type_path) else {
            if registry.is_ambiguous(type_path) {
                return Err(format!(
                    "`{type_path}` names more than one type; write its full path"
                ));
            }
            continue;
        };
        let reflected = bsn_value_to_reflect(&value, registration.type_id(), registry, assets)
            .ok_or_else(|| format!("{type_path}: value does not fit the type"))?;
        values.push(concrete(registration, reflected.as_ref())?);
    }
    Ok(values)
}

fn registration_of<'a>(registry: &'a TypeRegistry, path: &str) -> Option<&'a TypeRegistration> {
    registry
        .get_with_type_path(path)
        .or_else(|| registry.get_with_short_type_path(path))
}

fn concrete(
    registration: &TypeRegistration,
    value: &dyn PartialReflect,
) -> Result<Box<dyn Reflect>, String> {
    let path = registration.type_info().type_path();
    if let Some(from_reflect) = registration.data::<ReflectFromReflect>() {
        if let Some(concrete) = from_reflect.from_reflect(value) {
            return Ok(concrete);
        }
    }
    // A partial value: fill it over the type's default.
    let default = registration
        .data::<ReflectDefault>()
        .ok_or_else(|| format!("{path}: neither FromReflect nor Default"))?;
    let mut concrete = default.default();
    concrete
        .try_apply(value)
        .map_err(|e| format!("{path}: {e}"))?;
    Ok(concrete)
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
        let ast = parse_bsn(text).map_err(|e| AssetLoaderError::Bsn(format!("{path}: {e:?}")))?;
        let [root] = ast.roots[..] else {
            return Err(AssetLoaderError::Bsn(format!(
                "{path}: a graph document has one root entity, not {}",
                ast.roots.len()
            )));
        };
        let assets = BsnApplyAssets {
            server: &self.server,
            local: None,
        };
        graph_from_document(&ast, root, &self.registry.read(), Some(&assets))
            .map_err(|e| AssetLoaderError::Bsn(format!("{path}: {e}")))
    }

    fn extensions(&self) -> &[&str] {
        &["animgraph.bsn"]
    }
}
