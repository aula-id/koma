//! SDLC lane ceremony: graph shape at assess + post-approve finish cost.
//!
//! `express | standard | full` is frozen on the contract. Graph validation lives
//! here (and is re-exported via `decompose` for call-site stability). Integrate /
//! keeper / prompts consult lane for branch-ready vs merge pressure.

use super::graph::ChecklistNode;

/// Verification / finish lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Express,
    Standard,
    Full,
}

impl Lane {
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Express => "express",
            Self::Standard => "standard",
            Self::Full => "full",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "express" => Ok(Self::Express),
            "standard" => Ok(Self::Standard),
            "full" => Ok(Self::Full),
            other => Err(format!(
                "error: lane must be express|standard|full (got '{other}')"
            )),
        }
    }

    /// Default post-approve integrate path: leave mission branch ready (no merge).
    pub fn prefer_branch_only(self) -> bool {
        matches!(self, Self::Express)
    }

    /// When taking the branch-only path, mark mission **done** (good finish) vs
    /// stay in integrate waiting for a human merge.
    pub fn branch_ready_completes_mission(self) -> bool {
        matches!(self, Self::Express)
    }

    pub fn keeper_ship_hint(self) -> &'static str {
        match self {
            Self::Express => {
                "[SDLC keeper]\n\
                 All open nodes are sealed. Lane is express — if acceptance is green, \
                 call mission_integrate (branch-ready finish; no merge pressure)."
            }
            Self::Standard => {
                "[SDLC keeper]\n\
                 All open nodes are sealed. If acceptance is green and verify evidence \
                 is in place, call mission_integrate (FF/merge to frozen non-main target \
                 when clean; else branch left ready)."
            }
            Self::Full => {
                "[SDLC keeper]\n\
                 All open nodes are sealed. Full lane: ensure human gates are approved, \
                 verify evidence is complete, then call mission_integrate for the full \
                 integrate matrix."
            }
        }
    }

    pub fn execute_finish_hint(self) -> &'static str {
        match self {
            Self::Express => {
                "- Finish (express lane): when OPEN is empty and acceptance is green, call \
                 `mission_integrate` — default is **branch-ready done** (mission branch left \
                 for PR/manual merge; no auto-merge). main/master auto-merge stays blocked.\n"
            }
            Self::Standard => {
                "- Finish (standard lane): when OPEN is empty, acceptance green, leaves verified, \
                 call `mission_integrate` — FF/merge into frozen non-main target if clean; else \
                 branch left ready. main/master blocked.\n"
            }
            Self::Full => {
                "- Finish (full lane): human gates must be approved; per-leaf verify evidence \
                 required; then `mission_integrate` full matrix (clean WT, commits ahead, \
                 merge when safe).\n"
            }
        }
    }
}

/// Parse lane string; unknown → error (callers that default to standard should pass "standard").
#[allow(dead_code)]
pub fn parse(s: &str) -> Result<Lane, String> {
    Lane::parse(s)
}

pub fn prefer_branch_only(lane: &str) -> bool {
    Lane::parse(lane)
        .map(|l| l.prefer_branch_only())
        .unwrap_or(false)
}

pub fn branch_ready_completes_mission(lane: &str) -> bool {
    Lane::parse(lane)
        .map(|l| l.branch_ready_completes_mission())
        .unwrap_or(false)
}

pub fn keeper_ship_hint(lane: &str) -> &'static str {
    Lane::parse(lane)
        .map(|l| l.keeper_ship_hint())
        .unwrap_or(Lane::Standard.keeper_ship_hint())
}

pub fn execute_finish_hint(lane: &str) -> &'static str {
    Lane::parse(lane)
        .map(|l| l.execute_finish_hint())
        .unwrap_or(Lane::Standard.execute_finish_hint())
}

/// Validate lane against the proposed graph structure (assess / mission_ready).
///
/// - `express`: free form
/// - `standard`: hard-reject a single flat node with ≥3 acceptance criteria
/// - `full`: requires a tree (any parent link) OR ≥3 leaves
pub fn validate_lane_graph(
    lane: &str,
    nodes: &[ChecklistNode],
    acceptance_len: usize,
) -> Result<(), String> {
    let lane = Lane::parse(lane).unwrap_or(Lane::Standard);
    let leaf_count = count_leaves(nodes);
    let has_tree = nodes.iter().any(|n| n.parent_title.is_some());

    match lane {
        Lane::Express => Ok(()),
        Lane::Full => {
            if has_tree || leaf_count >= 3 {
                Ok(())
            } else {
                Err(
                    "error: full lane requires a hierarchical graph (parent links) \
                     or at least 3 leaf tasks"
                        .into(),
                )
            }
        }
        Lane::Standard => {
            if nodes.len() == 1 && !has_tree && acceptance_len >= 3 {
                Err(
                    "error: standard lane rejects a single megatask with ≥3 acceptance \
                     criteria — decompose into multiple leaf tasks or a hierarchy"
                        .into(),
                )
            } else {
                Ok(())
            }
        }
    }
}

fn count_leaves(nodes: &[ChecklistNode]) -> usize {
    let parents: std::collections::HashSet<&str> = nodes
        .iter()
        .filter_map(|n| n.parent_title.as_deref())
        .collect();
    nodes
        .iter()
        .filter(|n| !parents.contains(n.title.as_str()))
        .count()
}

#[cfg(test)]
#[path = "lane_test.rs"]
mod tests;
