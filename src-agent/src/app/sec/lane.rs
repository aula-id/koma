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

/// A capability a tool can require.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    /// Reads source, a corpus or an artifact. Always permitted.
    Read,
    /// Opens a connection to a target.
    Network,
    /// Starts something that listens (a listener, a server).
    Listen,
    /// Writes a proposed change.
    Propose,
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
            (_, Cap::Read) => true,
            (Self::Research, Cap::Network | Cap::Listen | Cap::Apply) => false,
            (Self::Research, Cap::Propose) => true,
            (Self::Red, Cap::Network | Cap::Listen) => true,
            // Red does not touch the tree; it reports.
            (Self::Red, Cap::Propose | Cap::Apply) => false,
            (Self::Blue, Cap::Network) => true,
            (Self::Blue, Cap::Propose) => true,
            (Self::Blue, Cap::Listen | Cap::Apply) => false,
        }
    }

    /// Why `cap` is refused, for the model to see.
    #[must_use]
    pub fn reason_for(self, cap: Cap) -> &'static str {
        match (self, cap) {
            (Self::Research, Cap::Network) => "research lane does not reach the network",
            (Self::Research, Cap::Listen) => "research lane does not start listeners",
            (Self::Research, Cap::Apply) => "research lane proposes, does not apply",
            (Self::Red, Cap::Propose) => "red lane reports, does not propose patches",
            (Self::Red, Cap::Apply) => "red lane does not write to the tree",
            (Self::Blue, Cap::Listen) => "blue lane does not start listeners",
            (Self::Blue, Cap::Apply) => "blue lane proposes a diff; the edit path applies it",
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
            "sec_decode" | "sec_triage" | "sec_sourcemap" | "sec_unmin" | "sec_jsdeobf"
            | "sec_wasm" | "sec_rop" | "sec_z3" | "sec_sage" | "sec_rsa" | "sec_lattice"
            | "sec_hashid" => Cap::Read,
            // These dial out.
            "sec_http" | "sec_remote" | "sec_ffuf" | "sec_nuclei" | "sec_dalfox"
            | "sec_sqlmap" | "sec_zap" | "sec_xss_confirm" | "sec_factor" => Cap::Network,
            _ if tool.starts_with("sec_") => Cap::Network,
            _ => Cap::Read,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lanes() -> [Lane; 3] {
        [Lane::Research, Lane::Red, Lane::Blue]
    }

    fn caps() -> [Cap; 5] {
        [Cap::Read, Cap::Network, Cap::Listen, Cap::Propose, Cap::Apply]
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
    fn blue_proposes_but_never_applies() {
        assert!(Lane::Blue.permits(Cap::Propose));
        assert!(!Lane::Blue.permits(Cap::Apply));
        assert!(!Lane::Red.permits(Cap::Propose));
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
        for tool in ["sec_http", "sec_ffuf", "sec_sqlmap", "sec_factor"] {
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
                ("research", vec![true, false, false, true, false]),
                ("red", vec![true, true, true, false, false]),
                ("blue", vec![true, true, false, true, false]),
            ]
        );
    }
}
