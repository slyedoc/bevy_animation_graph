//! `.animgraph.bsn` and `.fsm.bsn` documents build the graphs and state machines they describe.

use bevy::{bsn_document::parse_bsn, prelude::*};
use bevy_animation_graph_builtin_nodes::BuiltinNodesPlugin;
use bevy_animation_graph_core::{
    animation_graph::{
        SourcePin, TargetPin,
        bsn::{AnimGraph, Links, NodePosition, graph_from_document, node_id},
    },
    state_machine::high_level::bsn::{AnimFsm, AnimState, AnimTransition, fsm_from_document, state_id},
};

/// zero's one-shot `jump` state: the clip plays, and its finish event becomes a transition
/// request on `driver_events`.
const JUMP: &str = r#"
#jump
AnimGraph { outputs: [Time, Data("pose", Pose), Data("driver_events", EventQueue)] }
Links([
    Link(Data("pose"), Node("clip", "pose")),
    Link(Time(""), NodeTime("clip")),
    Link(Data("driver_events"), Node("on finish", "events")),
])
Children [
    #clip
    ClipNode { clip: "anim/ual/Jump_Start.anim.ron" }
    NodePosition(40.0, 40.0)
    --
    #"on finish"
    MapEventsNode { map_from: AnimationClipFinished, map_to: TransitionToStateLabel("airborne") }
    Links([Link(Data("events"), Node("clip", "events"))])
    NodePosition(330.0, 140.0)
]
"#;

fn registry() -> AppTypeRegistry {
    let mut app = App::new();
    app.add_plugins(BuiltinNodesPlugin)
        .register_type::<AnimGraph>()
        .register_type::<Links>()
        .register_type::<NodePosition>()
        .register_type::<AnimFsm>()
        .register_type::<AnimState>()
        .register_type::<AnimTransition>();
    app.world().resource::<AppTypeRegistry>().clone()
}

#[test]
fn a_bsn_graph_builds_its_nodes_edges_and_pins() {
    let ast = parse_bsn(JUMP).expect("parses");
    let registry = registry();
    let graph = graph_from_document(&ast, ast.roots[0], &registry.read(), None).expect("builds");

    assert_eq!(graph.nodes.len(), 2);
    let clip = node_id("clip");
    let finish = node_id("on finish");
    assert_eq!(graph.nodes[&clip].name, "clip");
    assert_eq!(
        graph.edges_inverted[&TargetPin::OutputData("pose".into())],
        SourcePin::NodeData(clip, "pose".into())
    );
    assert_eq!(
        graph.edges_inverted[&TargetPin::OutputTime],
        SourcePin::NodeTime(clip)
    );
    assert_eq!(
        graph.edges_inverted[&TargetPin::NodeData(finish, "events".into())],
        SourcePin::NodeData(clip, "events".into())
    );
    assert!(graph.io_spec.has_output_time());
    assert_eq!(
        graph.editor_metadata.node_positions[&finish],
        Vec2::new(330.0, 140.0)
    );
}

#[test]
fn graph_inputs_keep_their_order_and_defaults() {
    let doc = r#"
#g
AnimGraph {
    inputs: [
        Data { pin: Passthrough("speed"), spec: F32, default: Some(F32(1.5)) },
        Data { pin: FromFsmSource("pose"), spec: Pose },
        Time(FromFsmSource("time")),
    ],
    outputs: [Data("pose", Pose)],
}
Links([Link(Data("pose"), Input(FromFsmSource("pose")))])
"#;
    let ast = parse_bsn(doc).expect("parses");
    let graph = graph_from_document(&ast, ast.roots[0], &registry().read(), None).expect("builds");
    let inputs = graph.io_spec.sorted_inputs();
    assert_eq!(inputs.len(), 3, "{inputs:?}");
    assert_eq!(graph.default_data.len(), 1);
}

#[test]
fn a_node_without_a_node_component_is_refused() {
    let doc = "#g\nAnimGraph { }\nChildren [\n    #lost\n    NodePosition(1.0, 2.0)\n]\n";
    let ast = parse_bsn(doc).expect("parses");
    let err = graph_from_document(&ast, ast.roots[0], &registry().read(), None).unwrap_err();
    assert!(err.contains("lost"), "{err}");
}

const LOCOMOTION: &str = r#"
#locomotion
AnimFsm { start: "grounded", inputs: [Data("speed", F32)], outputs: [Time, Data("pose", Pose)] }
Children [
    #grounded
    AnimState { }
    NodePosition(0.0, 0.0)
    --
    #jump
    AnimState { enter: Some(TransitionData { kind: Graph { graph: "", timed: Some(0.08) }, reset_target_state: true }) }
    NodePosition(330.0, -150.0)
    --
    AnimTransition { from: "jump", to: "grounded" }
]
"#;

#[test]
fn a_bsn_state_machine_builds_its_states_and_transitions() {
    let ast = parse_bsn(LOCOMOTION).expect("parses");
    let fsm = fsm_from_document(&ast, ast.roots[0], &registry().read(), None).expect("builds");

    assert_eq!(fsm.states.len(), 2);
    assert_eq!(fsm.start_state, state_id("grounded"));
    let jump = &fsm.states[&state_id("jump")];
    assert_eq!(jump.label, "jump");
    let enter = jump.state_transition.as_ref().expect("jump has an entry transition");
    assert!(enter.reset_target_state);
    assert_eq!(fsm.transitions.len(), 1);
    let t = fsm.transitions.values().next().unwrap();
    assert_eq!((t.source, t.target), (state_id("jump"), state_id("grounded")));
    assert_eq!(fsm.editor_metadata.states[&state_id("jump")], Vec2::new(330.0, -150.0));
}

#[test]
fn a_start_state_that_is_not_a_state_is_refused() {
    let doc = LOCOMOTION.replace(r#"start: "grounded""#, r#"start: "flying""#);
    let ast = parse_bsn(&doc).expect("parses");
    let err = fsm_from_document(&ast, ast.roots[0], &registry().read(), None).unwrap_err();
    assert!(err.contains("flying"), "{err}");
}


