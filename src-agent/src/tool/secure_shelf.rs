//! `secure_shelf`: read-only access to what the secure room found.
//!
//! The room (`/security`) keeps its shelf in `<session>/room.sqlite`: claims,
//! evidence, verdicts, findings and the reports the Reporter writes. The coding
//! agent can look at all of it without joining the room: list the project's
//! secure sessions, browse a shelf, read one item, or read a report. It never
//! writes, so a live room is not disturbed.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde_json::{json, Value};

use super::{Tool, ToolCtx};
use crate::room::browse;
use crate::room::shelf::Shelf;

const MAX_ROWS: usize = 60;
const MAX_TEXT: usize = 12_000;

pub struct SecureShelf;

struct Found {
    uuid: String,
    name: String,
    dir: PathBuf,
}

fn bucket_of(session_dir: &Path) -> Option<String> {
    session_dir.parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()).filter(|s| !s.is_empty())
}

/// Sessions of this project that have a room database, newest first.
fn rooms(session_dir: &Path) -> Result<Vec<Found>> {
    let Some(hash) = bucket_of(session_dir) else {
        bail!("cannot tell which project this session belongs to");
    };
    let mut out = Vec::new();
    for row in crate::model::session_registry::list_by_pwd(&hash)? {
        let Ok(dir) = crate::model::store::session_dir(&hash, &row.uuid) else { continue };
        if crate::room::room_db_path(&dir).is_file() {
            out.push(Found { uuid: row.uuid, name: row.name, dir });
        }
    }
    Ok(out)
}

fn pick(found: Vec<Found>, wanted: Option<&str>) -> Result<Found> {
    if found.is_empty() {
        bail!("this project has no secure room session yet (start one with /security)");
    }
    let Some(w) = wanted.map(str::trim).filter(|w| !w.is_empty()) else {
        return found.into_iter().next().ok_or_else(|| anyhow::anyhow!("no secure room session"));
    };
    let lower = w.to_lowercase();
    let mut hits: Vec<Found> = found
        .into_iter()
        .filter(|f| f.uuid.starts_with(w) || f.name.to_lowercase().contains(&lower))
        .collect();
    match hits.len() {
        0 => bail!("no secure session matches `{w}`; call with action \"sessions\" to list them"),
        1 => Ok(hits.remove(0)),
        _ => bail!("`{w}` matches {} sessions; use a longer id prefix", hits.len()),
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}\n… cut at {max} characters; read a narrower key or use `prefix`")
}

fn text_arg<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

impl Tool for SecureShelf {
    fn name(&self) -> &'static str {
        "secure_shelf"
    }

    fn description(&self) -> &'static str {
        "Read what the secure room (/security) found in this project, without joining it. Read-only. \
         `sessions` lists secure sessions; `list` browses a shelf (claims, evidence, verdicts, findings); \
         `get` reads one item by key; `reports` lists reports and `report` reads one in full; \
         `goals` shows each goal's open, verified and returned counts. Defaults to the newest secure session."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["sessions", "list", "get", "reports", "report", "goals"], "description": "What to read. Default list."},
                "session": {"type": "string", "description": "Secure session id prefix or part of its name. Default: the newest one in this project."},
                "prefix": {"type": "string", "description": "list: only keys starting with this, e.g. claims/ or findings/."},
                "key": {"type": "string", "description": "get/report: the shelf key, e.g. claims/web/ssrf or reports/final."}
            }
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let session_dir = ctx.session_dir.as_deref().ok_or_else(|| anyhow::anyhow!("no active session"))?;
        let action = text_arg(args, "action").unwrap_or("list");
        let found = rooms(session_dir)?;
        if action == "sessions" {
            if found.is_empty() {
                return Ok("No secure room session in this project yet.".into());
            }
            let mut out = String::from("Secure sessions (newest first):\n");
            for f in &found {
                let name = if f.name.is_empty() { "(unnamed)" } else { f.name.as_str() };
                out.push_str(&format!("- {} {}\n", &f.uuid[..f.uuid.len().min(8)], name));
            }
            return Ok(out);
        }
        let room = pick(found, text_arg(args, "session"))?;
        let shelf = Shelf::open_read_only(&crate::room::room_db_path(&room.dir)).map_err(|e| anyhow::anyhow!("{e}"))?;
        let head = format!("secure session {} {}\n", &room.uuid[..room.uuid.len().min(8)], room.name);
        Ok(format!("{head}{}", render(&shelf, action, args)?))
    }
}

fn render(shelf: &Shelf, action: &str, args: &Value) -> Result<String> {
    {
        let body = match action {
            "list" => {
                let prefix = text_arg(args, "prefix").unwrap_or("");
                let rows: Vec<_> = browse::build(shelf).shelf.into_iter().filter(|r| r.key.starts_with(prefix)).collect();
                let total = rows.len();
                let mut out = format!("{total} shelf item(s){}\n", if prefix.is_empty() { String::new() } else { format!(" under {prefix}") });
                for r in rows.iter().take(MAX_ROWS) {
                    let status = if r.status.is_empty() { String::new() } else { format!(" [{}]", r.status) };
                    out.push_str(&format!("- {} r{} {}{} {}\n", r.key, r.revision, r.kind, status, r.title));
                }
                if total > MAX_ROWS {
                    out.push_str(&format!("… {} more; narrow with `prefix`\n", total - MAX_ROWS));
                }
                out
            }
            "get" => {
                let key = text_arg(args, "key").ok_or_else(|| anyhow::anyhow!("`key` is required"))?;
                let item = browse::item(shelf, key).ok_or_else(|| anyhow::anyhow!("no shelf item `{key}`"))?;
                format!("{} (revision {})\n{}", item.key, item.revision, clip(&item.content, MAX_TEXT))
            }
            "reports" => {
                let reports = browse::build(shelf).reports;
                if reports.is_empty() {
                    "No reports yet. The Reporter writes them once findings are verified.".to_string()
                } else {
                    let mut out = format!("{} report(s)\n", reports.len());
                    for r in reports.iter().take(MAX_ROWS) {
                        let verdict = if r.go_no_go.is_empty() { String::new() } else { format!(" · {}", r.go_no_go) };
                        out.push_str(&format!("- {} {} ({}){}\n", r.key, r.title, r.audience, verdict));
                    }
                    out
                }
            }
            "report" => {
                let reports = browse::build(shelf).reports;
                let report = match text_arg(args, "key") {
                    Some(key) => reports.into_iter().find(|r| r.key == key),
                    None => reports.into_iter().max_by_key(|r| r.updated),
                };
                let r = report.ok_or_else(|| anyhow::anyhow!("no such report; call with action \"reports\""))?;
                let summary = if r.severity_summary.is_empty() { String::new() } else { format!("{}\n", r.severity_summary) };
                format!("{} · {}\n{}{}", r.key, r.title, summary, clip(&r.markdown, MAX_TEXT))
            }
            "goals" => {
                let goals = browse::build(shelf).goals;
                if goals.is_empty() {
                    "No goals yet.".to_string()
                } else {
                    let mut out = String::new();
                    for g in &goals {
                        out.push_str(&format!("- {}: {} open, {} verified, {} returned, {} node(s)\n", g.goal_key, g.open, g.verified, g.returned, g.nodes.len()));
                    }
                    out
                }
            }
            other => bail!("unknown action `{other}`"),
        };
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_is_picked_by_id_prefix_or_name() {
        let f = |u: &str, n: &str| Found { uuid: u.into(), name: n.into(), dir: PathBuf::new() };
        let set = || vec![f("aaaa1111", "api review"), f("bbbb2222", "web review")];
        assert_eq!(pick(set(), None).unwrap().uuid, "aaaa1111");
        assert_eq!(pick(set(), Some("bbbb")).unwrap().uuid, "bbbb2222");
        assert_eq!(pick(set(), Some("API")).unwrap().uuid, "aaaa1111");
        assert!(pick(set(), Some("review")).is_err());
        assert!(pick(set(), Some("zzz")).is_err());
        assert!(pick(vec![], None).is_err());
    }

    fn shelf() -> Shelf {
        let shelf = Shelf::open_in_memory().expect("shelf");
        let allow: Vec<String> = ["put_claim", "put_report"].iter().map(|s| s.to_string()).collect();
        let put = |tool: &str, args: Value| shelf.call_as(&allow, tool, &args).unwrap_or_else(|e| panic!("{tool}: {e}"));
        put("put_claim", json!({ "key": "claims/web/ssrf", "vulnclass": "ssrf", "name": "SSRF in fetch", "severity": "high", "summary": "s", "host": "app", "poc": "curl", "expected_revision": 0 }));
        put("put_report", json!({ "key": "reports/final", "markdown": "# Final\n\nAll good.", "finding_key": "findings/x", "title": "Final report", "audience": "manager", "expected_revision": 0 }));
        shelf
    }

    #[test]
    fn the_agent_can_list_get_and_read_a_report() {
        let shelf = shelf();
        let say = |action: &str, args: Value| render(&shelf, action, &args).expect(action);
        assert!(say("list", json!({})).contains("claims/web/ssrf"));
        assert!(!say("list", json!({ "prefix": "reports/" })).contains("claims/"));
        assert!(say("get", json!({ "key": "claims/web/ssrf" })).contains("SSRF in fetch"));
        assert!(say("reports", json!({})).contains("Final report"));
        assert!(say("report", json!({})).contains("All good."));
        assert!(render(&shelf, "get", &json!({ "key": "nope" })).is_err());
        assert!(render(&shelf, "wipe", &json!({})).is_err());
    }

    #[test]
    fn long_text_is_cut_with_a_note() {
        assert_eq!(clip("short", 10), "short");
        assert!(clip(&"x".repeat(50), 10).contains("cut at 10"));
    }
}
