use super::*;

fn lanes() -> [Lane; 3] {
    [Lane::Research, Lane::Red, Lane::Blue]
}

fn caps() -> [Cap; 7] {
    [
        Cap::Read,
        Cap::Network,
        Cap::Observe,
        Cap::Listen,
        Cap::Propose,
        Cap::Edit,
        Cap::Apply,
    ]
}

#[test]
fn every_surface_entry_is_permitted_by_its_own_lane() {
    // The surface is generated into an agent definition, so a tool offered to a
    // lane its matrix forbids would be refused at dispatch after the model had
    // already tried it. This is the drift guard between the two lists.
    for lane in lanes() {
        for tool in surface(lane) {
            let cap = Lane::cap_of_tool(tool);
            assert!(
                lane.permits(cap),
                "{lane:?} surface offers {tool} but forbids {cap:?}"
            );
        }
    }
}

#[test]
fn research_gets_no_network_tool_at_all() {
    // The sharpest edge of the split: research must not be able to open a socket.
    for tool in surface(Lane::Research) {
        assert_eq!(Lane::cap_of_tool(tool), Cap::Read, "{tool} is not local");
    }
}

#[test]
fn blue_gets_no_listener() {
    for tool in surface(Lane::Blue) {
        assert_ne!(Lane::cap_of_tool(tool), Cap::Listen, "{tool} listens");
    }
}

#[test]
fn red_gets_the_target_path() {
    for tool in ["sec_http", "sec_browser", "sec_ffuf"] {
        assert!(surface(Lane::Red).contains(&tool), "{tool} missing from red");
    }
}

#[test]
fn every_surface_keeps_the_reading_tools() {
    // All three lanes read; a surface without them cannot work at all.
    for lane in lanes() {
        for tool in ["read", "grep", "glob", "dir_list"] {
            assert!(surface(lane).contains(&tool), "{lane:?} lost {tool}");
        }
    }
}

#[test]
fn every_lane_reads() {
    for lane in lanes() {
        assert!(lane.permits(Cap::Read), "{lane:?}");
    }
}

#[test]
fn research_never_reaches_the_network() {
    assert!(!Lane::Research.permits(Cap::Network));
    assert!(!Lane::Research.permits(Cap::Listen));
}

#[test]
fn only_red_listens() {
    assert!(Lane::Red.permits(Cap::Listen));
    assert!(!Lane::Blue.permits(Cap::Listen));
}

#[test]
fn blue_may_read_and_edit_and_may_not_probe() {
    assert!(Lane::Blue.permits(Cap::Observe));
    assert!(Lane::Blue.permits(Cap::Edit));
    assert!(!Lane::Blue.permits(Cap::Network));
    assert!(!Lane::Blue.permits(Cap::Propose));
    assert!(!Lane::Blue.permits(Cap::Apply));
    assert!(Lane::Red.permits(Cap::Network));
    assert!(!Lane::Red.permits(Cap::Propose));
}

#[test]
fn proof_tools_stay_on_red() {
    for tool in ["sec_browser", "sec_xss_confirm"] {
        assert!(surface(Lane::Red).contains(&tool), "{tool}");
        assert!(!surface(Lane::Blue).contains(&tool), "{tool}");
        assert_eq!(Lane::cap_of_tool(tool), Cap::Network);
    }
}

#[test]
fn every_refusal_has_a_reason() {
    for lane in lanes() {
        for cap in caps() {
            if !lane.permits(cap) {
                assert!(!lane.reason_for(cap).is_empty(), "{lane:?}/{cap:?}");
            }
        }
    }
}

#[test]
fn the_pure_tools_are_classified_read() {
    for tool in ["sec_decode", "sec_triage", "sec_z3", "sec_rop"] {
        assert_eq!(Lane::cap_of_tool(tool), Cap::Read, "{tool}");
    }
}

#[test]
fn the_dialling_tools_are_classified_network() {
    assert_eq!(Lane::cap_of_tool("sec_http"), Cap::Observe);
    for tool in ["sec_ffuf", "sec_sqlmap", "sec_factor", "sec_browser", "sec_xss_confirm"] {
        assert_eq!(Lane::cap_of_tool(tool), Cap::Network, "{tool}");
    }
}

#[test]
fn an_unknown_sec_tool_defaults_to_the_strict_side() {
    assert_eq!(Lane::cap_of_tool("sec_something_new"), Cap::Network);
}

#[test]
fn roles_map_to_lanes_and_non_cyber_roles_map_to_none() {
    use crate::model::app_config::ModelRole;
    assert_eq!(Lane::of_role(ModelRole::CyberRed), Some(Lane::Red));
    assert_eq!(Lane::of_role(ModelRole::Main), None);
}

#[test]
fn the_whole_matrix_is_what_the_table_claims() {
    // Pinned in one place so an accidental loosening shows up as a diff here.
    let matrix: Vec<(&str, Vec<bool>)> = lanes()
        .iter()
        .map(|l| (l.label(), caps().iter().map(|c| l.permits(*c)).collect()))
        .collect();
    assert_eq!(
        matrix,
        vec![
            ("research", vec![true, false, false, false, true, true, false]),
            ("red", vec![true, true, true, true, false, true, false]),
            ("blue", vec![true, false, true, false, false, true, false]),
        ]
    );
}
