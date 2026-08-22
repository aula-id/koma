//! SDLC historian: extracts edit audit candidates from completed tool calls
//! and produces best-effort purpose summaries via the Awareness model.

use super::graph::{EditAuditRecord, EditSummaryRecord};

/// Maximum characters for a historian-generated purpose string.
const PURPOSE_MAX_LEN: usize = 200;

/// Maximum characters for a single path in audit records.
const PATH_MAX_LEN: usize = 500;

/// A candidate audit record extracted from a completed tool call.
#[derive(Debug, Clone)]
pub struct EditAuditCandidate {
    pub tool: String,
    pub path: String,
    pub node_id: Option<String>,
    pub batch_id: String,
}

/// Tool names whose successful calls represent workspace mutations.
const EDIT_TOOLS: &[&str] = &["write", "edit", "delete"];

/// Extract an edit audit candidate from a completed tool call + result.
/// Returns None for non-edit tools, error results, or missing paths.
pub fn extract_edit_audit(
    call: &crate::dto::chat::ToolCall,
    result: &str,
    node_id: Option<&str>,
    batch_id: &str,
) -> Option<EditAuditCandidate> {
    let name = call.function.name.as_str();
    if !EDIT_TOOLS.contains(&name) {
        return None;
    }
    // Error results (including "error: ..." denials) are not successful mutations.
    if result.starts_with("error:") || result.starts_with("Error:") {
        return None;
    }
    let args: serde_json::Value = serde_json::from_str(&call.function.arguments).ok()?;
    let path = match name {
        "write" => args.get("path")?.as_str()?.to_string(),
        "edit" => args.get("path")?.as_str()?.to_string(),
        "delete" => args.get("path")?.as_str()?.to_string(),
        _ => return None,
    };
    if path.is_empty() || path.len() > PATH_MAX_LEN {
        return None;
    }
    Some(EditAuditCandidate {
        tool: name.to_string(),
        path,
        node_id: node_id.map(String::from),
        batch_id: batch_id.to_string(),
    })
}

/// Build the historian prompt messages for a batch of edits.
/// Returns (system, user) message pair for the Awareness model call.
pub fn build_historian_prompt(
    mission_goal: &str,
    mission_phase: &str,
    node_title: Option<&str>,
    audits: &[EditAuditRecord],
) -> (String, String) {
    let system = "You are an SDLC historian. Given a list of file edits made during an \
        SDLC mission, write ONE concise sentence (under 200 chars) describing the \
        PURPOSE of these edits in context of the mission goal.\n\
        Reply ONLY with valid JSON: {\"purpose\": \"<string under 200 chars>\"}\n\
        Be factual and specific. No preamble, no markdown."
        .to_string();

    let user = format!(
        "Mission goal: {mission_goal}\n\
         Phase: {mission_phase}\n\
         Task: {}\n\
         Edits (tool → path): {}\n\
         Reply JSON only.",
        node_title.unwrap_or("(no active task)"),
        audits
            .iter()
            .map(|a| format!("{} → {}", a.tool, a.path))
            .collect::<Vec<_>>()
            .join(", ")
    );

    (system, user)
}

/// Parse a historian model reply into an EditSummaryRecord.
/// Returns None for malformed/oversized responses.
pub fn parse_historian_reply(
    reply: &str,
    batch_id: &str,
    node_id: Option<&str>,
    paths: Vec<String>,
) -> Option<EditSummaryRecord> {
    let v: serde_json::Value = serde_json::from_str(reply).ok()?;
    let purpose = v.get("purpose")?.as_str()?;
    // Bound length.
    let purpose: String = purpose.chars().take(PURPOSE_MAX_LEN).collect();
    if purpose.trim().is_empty() {
        return None;
    }
    Some(EditSummaryRecord {
        batch_id: batch_id.to_string(),
        purpose,
        paths,
        node_id: node_id.map(String::from),
    })
}

#[cfg(test)]
#[path = "history_test.rs"]
mod tests;
