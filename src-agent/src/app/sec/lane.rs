//! What each cyber role may do. A matrix, not a prompt.
//!
//! The roles exist to constrain capability, so the constraint has to be checked at
//! dispatch and at the filesystem/network boundary — a sentence in a system prompt
//! is advice, not a boundary. Every rule here is enforced by
//! [`Lane::permits`] and by the tool-name gate that calls it.
//!
//! The asymmetry is deliberate and is the whole point of the split:
//!
//! | capability            | Research | Red | Blue |
//! |-----------------------|----------|-----|------|
//! | read source/corpus    | yes      | yes | yes  |
//! | reach the network     | no       | yes | yes  |
//! | run a target probe    | no       | yes | yes  |
//! | start a listener      | no       | yes | no   |
//! | propose a patch       | yes      | no  | yes  |
//! | apply a patch         | no       | no  | no   |
//!
//! `Research` reads without touching a target: it is the desk work lane, so a
//! forgotten tool must not silently open a socket. `Blue` may propose but not
//! apply, because the apply path is the normal Koma edit flow and a blue turn
//! going around it would bypass the user's diff review.

/// A cyber lane. Distinct from [`crate::model::app_config::ModelRole`]: the role
/// picks the model, the lane picks the rules. They are separate so a lane can be
/// re-assigned a model without changing what it may touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lane {
    Research,
    Red,
    Blue,
}

/// The tool surface each lane's agent definition is generated with.
///
/// Kept HERE, beside the matrix, so a capability change and the surface that
/// expresses it are one edit. The subset is deliberately narrower than what the
/// matrix permits: the matrix is the ceiling, the surface is the default, and the
/// dispatch gate enforces the ceiling either way.
///
/// `edit` is absent from every lane because blue proposes through the normal Koma
/// edit path rather than applying — so it is offered only when that path is open,
/// which the plan keeps as an explicit later step rather than a default.
pub const RESEARCH_SURFACE: &[&str] = &[
    "read", "grep", "glob", "dir_list",
    // Local analysis only; nothing in the matrix lets research open a socket.
    "sec_decode", "sec_triage", "sec_hashid", "sec_jsdeobf", "sec_unmin",
    "sec_sourcemap", "sec_wasm", "sec_rop", "sec_z3", "sec_sage", "sec_rsa",
    "sec_lattice",
    // NOT sec_crack / sec_factor: both LOOK local but the daemon's effect manifest
    // ties them to an external fetch (a hashcat download, a fixed factordb.com
    // request), so `cap_of_tool` classifies both `Network` and research is refused
    // them. The surface follows the manifest, not the appearance.
];

pub const RED_SURFACE: &[&str] = &[
    "read", "grep", "glob", "dir_list",
    // The target path: everything that dials out, plus the browser.
    "sec_http", "sec_browser", "sec_nuclei", "sec_ffuf", "sec_dalfox", "sec_sqlmap",
    "sec_zap", "sec_xss_confirm", "sec_remote", "sec_replay", "sec_recording",
    // Local analysis, because a red turn reads what it just pulled back.
    "sec_decode", "sec_triage", "sec_hashid", "sec_jsdeobf", "sec_unmin",
    "sec_sourcemap", "sec_wasm", "sec_rop", "sec_z3", "sec_sage", "sec_rsa",
    "sec_lattice", "sec_crack", "sec_factor",
];

pub const BLUE_SURFACE: &[&str] = &[
    "read", "grep", "glob", "dir_list",
    "sec_decode", "sec_triage", "sec_hashid", "sec_jsdeobf", "sec_unmin",
    "sec_sourcemap", "sec_wasm", "sec_rop", "sec_z3", "sec_sage", "sec_rsa",
    "sec_lattice",
    "sec_http",
    "edit", "write",
];

/// The surface for a lane.
#[must_use]
pub fn surface(lane: Lane) -> &'static [&'static str] {
    match lane {
        Lane::Research => RESEARCH_SURFACE,
        Lane::Red => RED_SURFACE,
        Lane::Blue => BLUE_SURFACE,
    }
}

/// A capability a tool can require.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    /// Reads source, a corpus or an artifact. Always permitted.
    Read,
    /// A probe, a scanner, or any request that is not a bare read.
    Network,
    /// `GET` or `HEAD` of a bare URL.
    Observe,
    /// Starts something that listens (a listener, a server).
    Listen,
    /// Writes a proposed change.
    Propose,
    /// Saves a file with `edit` or `write`.
    Edit,
    /// Writes into the working tree for real.
    Apply,
}

impl Lane {
    /// Parse the lane a model role maps to, or `None` for a non-cyber role.
    #[must_use]
    pub fn of_role(role: crate::model::app_config::ModelRole) -> Option<Self> {
        use crate::model::app_config::ModelRole;
        match role {
            ModelRole::CyberResearch => Some(Self::Research),
            ModelRole::CyberRed => Some(Self::Red),
            ModelRole::CyberBlue => Some(Self::Blue),
            _ => None,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Research => "research",
            Self::Red => "red",
            Self::Blue => "blue",
        }
    }

    /// The global role that picks this lane's model.
    ///
    /// One-way on purpose: [`Self::of_role`] is the inverse, and both are pinned
    /// by a test so a swap cannot silently gate the wrong capability.
    #[must_use]
    pub fn role(self) -> crate::model::app_config::ModelRole {
        use crate::model::app_config::ModelRole;
        match self {
            Self::Research => ModelRole::CyberResearch,
            Self::Red => ModelRole::CyberRed,
            Self::Blue => ModelRole::CyberBlue,
        }
    }

    /// May this lane perform `cap`?
    #[must_use]
    pub fn permits(self, cap: Cap) -> bool {
        match (self, cap) {
            (_, Cap::Read | Cap::Edit) => true,
            (Self::Research, Cap::Propose) => true,
            (Self::Research, Cap::Network | Cap::Observe | Cap::Listen | Cap::Apply) => false,
            (Self::Red, Cap::Network | Cap::Observe | Cap::Listen) => true,
            (Self::Red, Cap::Propose | Cap::Apply) => false,
            (Self::Blue, Cap::Observe) => true,
            (Self::Blue, Cap::Network | Cap::Listen | Cap::Propose | Cap::Apply) => false,
        }
    }

    /// Why `cap` is refused, for the model to see.
    #[must_use]
    pub fn reason_for(self, cap: Cap) -> &'static str {
        match (self, cap) {
            (Self::Research, Cap::Network | Cap::Observe) => {
                "research lane does not reach the network"
            }
            (Self::Research, Cap::Listen) => "research lane does not start listeners",
            (Self::Research, Cap::Apply) => "research lane proposes, does not apply",
            (Self::Red, Cap::Propose) => "red lane reports, does not propose patches",
            (Self::Red, Cap::Apply) => "red lane does not write to the tree",
            (Self::Blue, Cap::Network) => "blue lane does not probe",
            (Self::Blue, Cap::Listen) => "blue lane does not start listeners",
            (Self::Blue, Cap::Propose | Cap::Apply) => "blue lane writes with edit or write",
            _ => "not permitted in this lane",
        }
    }

    /// The capability a tool name implies, from its namespace.
    ///
    /// `sec_` predates the lanes, so it is classified per tool by what it does
    /// rather than by prefix. An unknown `sec_` tool defaults to [`Cap::Network`]
    /// — the strict side, since the alternative is assuming a network tool is
    /// free and letting the research lane open a socket.
    #[must_use]
    pub fn cap_of_tool(tool: &str) -> Cap {
        match tool {
            // Pure compute / local file inspection: no socket.
            "edit" | "write" => Cap::Edit,
            "sec_http" => Cap::Observe,
            "sec_decode" | "sec_triage" | "sec_sourcemap" | "sec_unmin" | "sec_jsdeobf"
            | "sec_wasm" | "sec_rop" | "sec_z3" | "sec_sage" | "sec_rsa" | "sec_lattice"
            | "sec_hashid" => Cap::Read,
            "sec_browser" | "sec_remote" | "sec_ffuf" | "sec_nuclei" | "sec_dalfox"
            | "sec_sqlmap" | "sec_zap" | "sec_xss" | "sec_xss_confirm" | "sec_replay"
            | "sec_factor" => Cap::Network,
            _ if tool.starts_with("sec_") => Cap::Network,
            _ => Cap::Read,
        }
    }

    /// Shell, delete, download, and the host browser. Blue refuses these by name.
    /// They stay [`Cap::Read`] so a red call is unchanged.
    #[must_use]
    pub fn blue_blocks(tool: &str) -> bool {
        matches!(
            tool,
            "bash"
                | "delete"
                | "git_operator"
                | "web_download"
                | "browser_tabs"
                | "browser_interact"
                | "browser_evaluate"
        )
    }
}

#[cfg(test)]
mod tests {
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
}

/// Build the agent definition for a lane.
///
/// Generated rather than hand-written so the tool list IS the constant above: a
/// hand-maintained markdown file would drift the moment the matrix moved, and the
/// drift would be invisible until the model tried a tool it could not use.
#[cfg(test)]
pub(crate) fn definition_for(lane: Lane) -> crate::model::agent_def::AgentDef {
    use crate::model::agent_def::AgentDef;

    let mut def = AgentDef {
        name: format!("cyber-{}", lane.label()),
        description: match lane {
            Lane::Research => "Cyber research lane: reads source and artifacts, proposes hypotheses.".into(),
            Lane::Red => "Cyber red lane: tests hypotheses against the in-scope target.".into(),
            Lane::Blue => "Cyber blue lane: turns confirmed findings into a proposed fix.".into(),
        },
        conditions: match lane {
            Lane::Research => "Delegating desk work: reading code, decoding, forming hypotheses.".into(),
            Lane::Red => "Testing a hypothesis against a target inside the recorded scope.".into(),
            Lane::Blue => "Writing and re-verifying a fix for a confirmed finding.".into(),
        },
        ..Default::default()
    };
    def.tools = surface(lane).iter().map(|t| (*t).to_string()).collect();
    def
}

#[cfg(test)]
mod definitions {
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
}
