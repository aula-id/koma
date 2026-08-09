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
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn make_call(name: &str, args: &str) -> crate::dto::chat::ToolCall {
        crate::dto::chat::ToolCall {
            id: "call-1".into(),
            kind: "function".into(),
            function: crate::dto::chat::FunctionCall {
                name: name.into(),
                arguments: args.into(),
            },
        }
    }

    #[test]
    fn extract_write_success() {
        let call = make_call("write", r#"{"path":"src/foo.rs","content":"x"}"#);
        let candidate = extract_edit_audit(&call, "ok", Some("t1"), "batch-1").unwrap();
        assert_eq!(candidate.tool, "write");
        assert_eq!(candidate.path, "src/foo.rs");
        assert_eq!(candidate.node_id.as_deref(), Some("t1"));
        assert_eq!(candidate.batch_id, "batch-1");
    }

    #[test]
    fn extract_edit_success() {
        let call = make_call("edit", r#"{"path":"src/bar.rs","old":"a","new":"b"}"#);
        let candidate = extract_edit_audit(&call, "ok", None, "batch-2").unwrap();
        assert_eq!(candidate.tool, "edit");
        assert_eq!(candidate.path, "src/bar.rs");
        assert!(candidate.node_id.is_none());
    }

    #[test]
    fn extract_delete_success() {
        let call = make_call("delete", r#"{"path":"src/old.rs"}"#);
        let candidate = extract_edit_audit(&call, "deleted", Some("t2"), "batch-3").unwrap();
        assert_eq!(candidate.tool, "delete");
    }

    #[test]
    fn extract_skips_error_result() {
        let call = make_call("write", r#"{"path":"src/x.rs","content":"y"}"#);
        assert!(extract_edit_audit(&call, "error: denied", Some("t1"), "b1").is_none());
    }

    #[test]
    fn extract_skips_non_edit_tool() {
        let call = make_call("read", r#"{"path":"src/x.rs"}"#);
        assert!(extract_edit_audit(&call, "ok", None, "b1").is_none());
    }

    #[test]
    fn extract_skips_missing_path() {
        let call = make_call("write", r#"{"content":"x"}"#);
        assert!(extract_edit_audit(&call, "ok", None, "b1").is_none());
    }

    #[test]
    fn parse_historian_valid() {
        let reply = r#"{"purpose":"Added error handling to the authentication module"}"#;
        let rec =
            parse_historian_reply(reply, "b1", Some("t1"), vec!["src/auth.rs".into()]).unwrap();
        assert_eq!(rec.batch_id, "b1");
        assert_eq!(
            rec.purpose,
            "Added error handling to the authentication module"
        );
        assert_eq!(rec.paths, vec!["src/auth.rs"]);
        assert_eq!(rec.node_id.as_deref(), Some("t1"));
    }

    #[test]
    fn parse_historian_truncates_long_purpose() {
        let long_purpose = "x".repeat(500);
        let reply = format!(r#"{{"purpose":"{long_purpose}"}}"#);
        let rec = parse_historian_reply(&reply, "b1", None, vec![]).unwrap();
        assert!(rec.purpose.len() <= PURPOSE_MAX_LEN);
    }

    #[test]
    fn parse_historian_rejects_empty_purpose() {
        let reply = r#"{"purpose":"  "}"#;
        assert!(parse_historian_reply(reply, "b1", None, vec![]).is_none());
    }

    #[test]
    fn parse_historian_rejects_malformed() {
        assert!(parse_historian_reply("not json", "b1", None, vec![]).is_none());
        assert!(parse_historian_reply(r#"{"other":"x"}"#, "b1", None, vec![]).is_none());
    }

    #[test]
    fn build_prompt_contains_goal_and_paths() {
        let audits = vec![
            EditAuditRecord {
                tool: "write".into(),
                path: "src/a.rs".into(),
                node_id: Some("t1".into()),
                batch_id: "b1".into(),
            },
            EditAuditRecord {
                tool: "edit".into(),
                path: "src/b.rs".into(),
                node_id: Some("t1".into()),
                batch_id: "b1".into(),
            },
        ];
        let (system, user) =
            build_historian_prompt("ship feature X", "execute", Some("implement"), &audits);
        assert!(system.contains("SDLC historian"));
        assert!(user.contains("ship feature X"));
        assert!(user.contains("execute"));
        assert!(user.contains("implement"));
        assert!(user.contains("src/a.rs"));
        assert!(user.contains("src/b.rs"));
    }
}
