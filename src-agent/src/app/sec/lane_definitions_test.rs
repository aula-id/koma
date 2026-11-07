//! The generated definitions are what a model actually sees, so they are tested
//! as artefacts rather than trusted from the constants.

use super::*;

#[test]
fn each_lane_gets_a_named_definition() {
    for lane in [Lane::Research, Lane::Red, Lane::Blue] {
        assert_eq!(definition_for(lane).name, format!("cyber-{}", lane.label()));
    }
}

#[test]
fn the_definition_carries_the_surface_verbatim() {
    for lane in [Lane::Research, Lane::Red, Lane::Blue] {
        assert_eq!(definition_for(lane).tools, surface(lane));
    }
}

#[test]
fn the_allow_list_survives_effective_tools_untouched() {
    // `effective_tools` strips delegation tools; the surfaces carry none, so the
    // list must come through whole. If that ever changes, this fails rather than
    // the model silently losing a capability.
    for lane in [Lane::Research, Lane::Red, Lane::Blue] {
        let def = definition_for(lane);
        assert_eq!(def.effective_tools(), def.tools);
    }
}

#[test]
fn the_definitions_are_not_empty_and_have_conditions() {
    // An empty `tools` would silently fall back to the read-only default, and an
    // empty `conditions` would leave the agent un-delegatable.
    for lane in [Lane::Research, Lane::Red, Lane::Blue] {
        let def = definition_for(lane);
        assert!(!def.effective_tools().is_empty());
        assert!(!def.conditions.trim().is_empty());
        assert!(!def.description.trim().is_empty());
    }
}

#[test]
fn a_definition_round_trips_through_markdown() {
    // These are meant to be writable to disk as normal agent files.
    let def = definition_for(Lane::Red);
    let markdown = def.to_markdown();
    assert!(markdown.contains("sec_http"));
    let back = crate::model::agent_def::parse_agent(
        "cyber-red",
        &markdown,
        crate::model::agent_def::AgentSource::Global,
    )
    .expect("parses");
    assert_eq!(back.tools, def.tools);
}
