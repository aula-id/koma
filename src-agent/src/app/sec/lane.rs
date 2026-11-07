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

/// Meta control-plane tools available on every lane when security mode is on.
pub const META_SURFACE: &[&str] = &["sec_catalog", "sec_run", "sec_fs", "sec_runtime"];

/// The surface for a lane (includes [`META_SURFACE`]).
#[must_use]
pub fn surface(lane: Lane) -> &'static [&'static str] {
    match lane {
        Lane::Research => research_surface(),
        Lane::Red => red_surface(),
        Lane::Blue => blue_surface(),
    }
}

fn research_surface() -> &'static [&'static str] {
    use std::sync::OnceLock;
    static S: OnceLock<Vec<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        let mut v = Vec::with_capacity(META_SURFACE.len() + RESEARCH_SURFACE.len());
        v.extend_from_slice(META_SURFACE);
        v.extend_from_slice(RESEARCH_SURFACE);
        v
    }).as_slice()
}

fn red_surface() -> &'static [&'static str] {
    use std::sync::OnceLock;
    static S: OnceLock<Vec<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        let mut v = Vec::with_capacity(META_SURFACE.len() + RED_SURFACE.len());
        v.extend_from_slice(META_SURFACE);
        v.extend_from_slice(RED_SURFACE);
        v
    }).as_slice()
}

fn blue_surface() -> &'static [&'static str] {
    use std::sync::OnceLock;
    static S: OnceLock<Vec<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        let mut v = Vec::with_capacity(META_SURFACE.len() + BLUE_SURFACE.len());
        v.extend_from_slice(META_SURFACE);
        v.extend_from_slice(BLUE_SURFACE);
        v
    }).as_slice()
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
            // Control-plane meta tools: discovery + jail lifecycle, not network.
            "sec_catalog" | "sec_runtime" | "sec_fs" | "sec_run" => Cap::Read,
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
#[path = "lane_test.rs"]
mod tests;


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
#[path = "lane_definitions_test.rs"]
mod definitions;

