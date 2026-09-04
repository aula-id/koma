#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::{is_sdlc_lifecycle_tool, mode_advertised_lifecycle_tools};
use crate::app::state::AgentMode;

#[test]
fn auto_normal_yolo_advertise_plan_enter_only() {
    for mode in [AgentMode::Auto, AgentMode::Normal, AgentMode::Yolo] {
        let tools = mode_advertised_lifecycle_tools(mode, None);
        assert_eq!(tools, vec!["plan_enter"], "mode={mode:?}");
    }
}

#[test]
fn plan_advertises_seqthink_and_plan_ready() {
    let tools = mode_advertised_lifecycle_tools(AgentMode::Plan, None);
    assert_eq!(tools, vec!["seqthink", "plan_ready"]);
}

#[test]
fn sdlc_assess_advertises_seqthink_and_mission_ready() {
    let tools = mode_advertised_lifecycle_tools(AgentMode::Sdlc, Some("assess"));
    assert_eq!(
        tools,
        vec!["seqthink", "mission_draft", "mission_ready"]
    );
}

#[test]
fn sdlc_prepare_advertises_mission_ready_and_mission_prepare() {
    let tools = mode_advertised_lifecycle_tools(AgentMode::Sdlc, Some("prepare"));
    assert_eq!(tools, vec!["mission_ready", "mission_prepare"]);
}

#[test]
fn sdlc_execute_advertises_mission_ready_verify_integrate() {
    let tools = mode_advertised_lifecycle_tools(AgentMode::Sdlc, Some("execute"));
    assert_eq!(
        tools,
        vec!["mission_ready", "mission_verify", "mission_integrate"]
    );
}

#[test]
fn sdlc_integrate_advertises_mission_ready_verify_integrate() {
    let tools = mode_advertised_lifecycle_tools(AgentMode::Sdlc, Some("integrate"));
    assert_eq!(
        tools,
        vec!["mission_ready", "mission_verify", "mission_integrate"]
    );
}

#[test]
fn sdlc_done_paused_none_advertise_mission_ready_only() {
    for phase in [None, Some("done"), Some("paused"), Some("unknown")] {
        let tools = mode_advertised_lifecycle_tools(AgentMode::Sdlc, phase);
        assert_eq!(tools, vec!["mission_ready"], "phase={phase:?}");
    }
}

/// No non-SDLC mode ever advertises a mission_* tool.
#[test]
fn no_mission_tools_outside_sdlc() {
    for mode in [
        AgentMode::Auto,
        AgentMode::Normal,
        AgentMode::Yolo,
        AgentMode::Plan,
    ] {
        let tools = mode_advertised_lifecycle_tools(mode, None);
        for t in &tools {
            assert!(
                !t.starts_with("mission_"),
                "mode={mode:?} must not advertise mission tool: {t}"
            );
        }
    }
}

/// Plan never advertises mission_* or plan_enter.
#[test]
fn plan_never_advertises_mission_or_plan_enter() {
    let tools = mode_advertised_lifecycle_tools(AgentMode::Plan, None);
    for t in &tools {
        assert!(
            !t.starts_with("mission_"),
            "plan must not advertise mission tools: {t}"
        );
        assert_ne!(*t, "plan_enter", "plan must not advertise plan_enter");
    }
}

/// SDLC assess never advertises mission_verify/prepare/integrate.
#[test]
fn sdlc_assess_no_verify_prepare_integrate() {
    let tools = mode_advertised_lifecycle_tools(AgentMode::Sdlc, Some("assess"));
    for t in &tools {
        assert_ne!(*t, "mission_verify", "assess must not advertise verify");
        assert_ne!(*t, "mission_prepare", "assess must not advertise prepare");
        assert_ne!(
            *t, "mission_integrate",
            "assess must not advertise integrate"
        );
    }
}

/// SDLC prepare never advertises mission_verify or mission_integrate.
#[test]
fn sdlc_prepare_no_verify_integrate() {
    let tools = mode_advertised_lifecycle_tools(AgentMode::Sdlc, Some("prepare"));
    for t in &tools {
        assert_ne!(*t, "mission_verify", "prepare must not advertise verify");
        assert_ne!(
            *t, "mission_integrate",
            "prepare must not advertise integrate"
        );
    }
}

/// SDLC execute/integrate never advertises mission_prepare.
#[test]
fn sdlc_execute_integrate_no_prepare() {
    for phase in ["execute", "integrate"] {
        let tools = mode_advertised_lifecycle_tools(AgentMode::Sdlc, Some(phase));
        for t in &tools {
            assert_ne!(*t, "mission_prepare", "{phase} must not advertise prepare");
        }
    }
}

/// Every lifecycle tool is tracked in SDLC_LIFECYCLE_TOOLS.
#[test]
fn all_mission_tools_in_sdlc_lifecycle_set() {
    for name in [
        "mission_ready",
        "mission_draft",
        "mission_verify",
        "mission_prepare",
        "mission_integrate",
    ] {
        assert!(
            is_sdlc_lifecycle_tool(name),
            "{name} must be in SDLC_LIFECYCLE_TOOLS"
        );
    }
    // Non-lifecycle tools must NOT be flagged.
    assert!(!is_sdlc_lifecycle_tool("plan_enter"));
    assert!(!is_sdlc_lifecycle_tool("seqthink"));
    assert!(!is_sdlc_lifecycle_tool("plan_ready"));
}

/// No tool returned by any mode/phase is duplicated.
#[test]
fn no_duplicates_across_any_mode_phase() {
    let modes_and_phases: Vec<(AgentMode, Option<&str>)> = vec![
        (AgentMode::Auto, None),
        (AgentMode::Normal, None),
        (AgentMode::Yolo, None),
        (AgentMode::Plan, None),
        (AgentMode::Sdlc, Some("assess")),
        (AgentMode::Sdlc, Some("prepare")),
        (AgentMode::Sdlc, Some("execute")),
        (AgentMode::Sdlc, Some("integrate")),
        (AgentMode::Sdlc, Some("done")),
        (AgentMode::Sdlc, None),
    ];
    for (mode, phase) in modes_and_phases {
        let tools = mode_advertised_lifecycle_tools(mode, phase);
        let mut seen = std::collections::HashSet::new();
        for t in &tools {
            assert!(
                seen.insert(*t),
                "duplicate tool '{t}' for mode={mode:?} phase={phase:?}"
            );
        }
    }
}
