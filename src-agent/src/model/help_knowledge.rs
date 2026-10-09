//! Offline product reference shared with the GUI.
use include_dir::{include_dir, Dir};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
static HELP: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../src-misc/help");
const MANIFEST_JSON: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../src-misc/help/manifest.json"));
#[derive(Debug, Clone, Deserialize)]
pub struct Article {
    pub id: String,
    pub title: String,
    pub aliases: Vec<String>,
    pub navigation: String,
    pub workflows: Vec<String>,
    pub file: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub articles: Vec<Article>,
    pub navigation: Vec<String>,
    pub workflows: Vec<String>,
}
pub fn manifest() -> Result<Manifest, String> {
    static MANIFEST: OnceLock<Result<Manifest, String>> = OnceLock::new();
    MANIFEST
        .get_or_init(|| {
            serde_json::from_str(MANIFEST_JSON)
                .map_err(|e| format!("bundled help/manifest.json: {e}"))
        })
        .clone()
}
pub fn article(id: &str) -> Option<String> {
    let m = manifest().ok()?;
    let a = m.articles.iter().find(|a| a.id == id)?;
    Some(HELP.get_file(&a.file)?.contents_utf8()?.to_string())
}
pub fn ranked(query: &str, limit: usize) -> Vec<String> {
    let q_lower = query.to_lowercase();
    // GUI Help is the default surface; only pull TUI slash-command articles when
    // the user is clearly asking about the terminal product.
    let want_tui = ["tui", "terminal", "slash", "keybind", "hotkey", "/help", "/settings", "/model"]
        .iter()
        .any(|k| q_lower.contains(k));
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(str::to_lowercase)
        .collect();
    let Ok(m) = manifest() else {
        return Vec::new();
    };
    let mut scores: Vec<_> = m
        .articles
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let title = format!("{} {}", a.title, a.aliases.join(" ")).to_lowercase();
            let body = article(&a.id).unwrap_or_default().to_lowercase();
            let mut score = words
                .iter()
                .map(|w| {
                    if title.contains(w) {
                        8
                    } else if body.contains(w) {
                        1
                    } else {
                        0
                    }
                })
                .sum::<usize>();
            let is_tui = a.id.starts_with("tui-");
            if !want_tui && is_tui {
                // Demote TUI-only articles for GUI-facing questions.
                score /= 4;
            } else if !want_tui && !is_tui && score > 0 {
                score += 2;
            } else if want_tui && is_tui && score > 0 {
                score += 4;
            }
            (score, i, a.id.clone())
        })
        .collect();
    scores.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    // Prefer positive matches; fall back to top catalogue order only if nothing scored.
    let positive: Vec<_> = scores.iter().filter(|s| s.0 > 0).cloned().collect();
    let pick = if positive.is_empty() { scores } else { positive };
    pick.into_iter().take(limit).map(|s| s.2).collect()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Answer {
    pub answer: String,
    #[serde(default)]
    pub articles: Vec<String>,
    #[serde(default)]
    pub navigation: Option<String>,
    #[serde(default)]
    pub guide: Option<String>,
}
/// Pull a JSON object out of model output that may include fences or prose.
pub fn extract_json_object(raw: &str) -> Option<String> {
    let mut t = raw.trim();
    if let Some(rest) = t.strip_prefix("```") {
        let rest = rest
            .strip_prefix("json")
            .or_else(|| rest.strip_prefix("JSON"))
            .unwrap_or(rest)
            .trim_start_matches(['\r', '\n', ' ']);
        t = rest.split("```").next().unwrap_or(rest).trim();
    }
    if t.starts_with('{') {
        // Trim trailing prose after a balanced top-level object when present.
        if let Some(end) = find_top_level_object_end(t) {
            return Some(t[..=end].trim().to_string());
        }
        return Some(t.to_string());
    }
    let start = t.find('{')?;
    let slice = &t[start..];
    let end = find_top_level_object_end(slice)?;
    Some(slice[..=end].trim().to_string())
}
fn find_top_level_object_end(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    for (i, ch) in s.char_indices() {
        if in_str {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        match ch {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}
pub fn parse_answer(raw: &str) -> Result<Answer, String> {
    let payload = extract_json_object(raw).ok_or_else(|| {
        "Help returned malformed guidance. Retry, or use Reference and Guides offline.".to_string()
    })?;
    // Ignore unknown keys from weak free-tier models; keep only Answer fields.
    let a: Answer = serde_json::from_str(&payload).map_err(|_| {
        "Help returned malformed guidance. Retry, or use Reference and Guides offline.".to_string()
    })?;
    let m = manifest()?;
    if a.answer.trim().is_empty()
        || a.answer.len() > 16000
        || a.articles.len() > 8
        || a.articles
            .iter()
            .any(|id| !m.articles.iter().any(|art| &art.id == id))
        || a.navigation
            .as_ref()
            .is_some_and(|id| !m.navigation.contains(id))
        || a.guide.as_ref().is_some_and(|id| !m.workflows.contains(id))
    {
        return Err(
            "Help returned an unknown topic or action. Retry, or use Reference and Guides.".into(),
        );
    }
    Ok(a)
}
/// Canonical JSON text for a validated answer (stable for multi-turn history).
pub fn answer_json(answer: &Answer) -> String {
    serde_json::to_string(answer).unwrap_or_else(|_| {
        format!(
            r#"{{"answer":{},"articles":[]}}"#,
            serde_json::to_string(&answer.answer).unwrap_or_else(|_| "\"\"".into())
        )
    })
}
/// Discard every unrecognized field and constrain strings to shipped IDs.
pub fn redact_context(input: &serde_json::Value) -> serde_json::Value {
    let m = manifest().ok();
    let empty: Vec<String> = Vec::new();
    let navigation = m.as_ref().map(|m| m.navigation.as_slice()).unwrap_or(&empty);
    let workflows = m.as_ref().map(|m| m.workflows.as_slice()).unwrap_or(&empty);
    let mut out = serde_json::Map::new();
    for key in [
        "attached",
        "hasProviders",
        "hasModels",
        "hasMcp",
        "hasSearchConfig",
        "hasWorkspace",
        "remote",
    ] {
        if let Some(v) = input.get(key).and_then(|v| v.as_bool()) {
            out.insert(key.into(), v.into());
        }
    }
    if let Some(p) = input
        .get("platform")
        .and_then(|v| v.as_str())
        .filter(|p| ["mac", "windows", "linux", "unknown"].contains(p))
    {
        out.insert("platform".into(), p.into());
    }
    for (key, ids) in [("activeView", navigation), ("guide", workflows)] {
        if let Some(v) = input
            .get(key)
            .and_then(|v| v.as_str())
            .filter(|id| ids.iter().any(|known| known == id))
        {
            out.insert(key.into(), v.into());
        }
    }
    if let Some(step) = input
        .get("guideStep")
        .and_then(|v| v.as_u64())
        .filter(|n| *n < 100)
    {
        out.insert("guideStep".into(), step.into());
    }
    if let Some(caps) = input.get("capabilities").and_then(|v| v.as_array()) {
        out.insert(
            "capabilities".into(),
            serde_json::Value::Array(
                caps.iter()
                    .filter_map(|v| {
                        v.as_str()
                            .filter(|id| navigation.iter().any(|known| known == id))
                    })
                    .map(|id| id.into())
                    .collect(),
            ),
        );
    }
    out.into()
}
/// Tool-free completion with bounded, host-owned article retrieval.
pub fn grounded_reply(
    messages: &mut Vec<serde_json::Value>,
    mut complete: impl FnMut(&[serde_json::Value]) -> Result<String, String>,
) -> Result<String, String> {
    for round in 0..=2 {
        let raw = complete(messages)?;
        let payload = extract_json_object(&raw).ok_or_else(|| {
            "Help returned malformed guidance. Retry, or use Reference and Guides offline."
                .to_string()
        })?;
        let value: serde_json::Value = serde_json::from_str(&payload).map_err(|_| {
            "Help returned malformed guidance. Retry, or use Reference and Guides offline."
                .to_string()
        })?;
        if let Some(ids) = value.get("request_articles") {
            if round == 2 {
                return Err("Help exceeded the article lookup limit. Retry with a more specific question or use Reference.".into());
            }
            // Allow extra keys alongside request_articles from sloppy models, but
            // only honor the article request when that is the intent.
            if value.get("answer").is_some() {
                // Prefer final answer if both shapes appear.
                let answer = parse_answer(&payload)?;
                return Ok(answer_json(&answer));
            }
            let ids = ids
                .as_array()
                .filter(|ids| !ids.is_empty() && ids.len() <= 4)
                .ok_or("Invalid Help article request")?;
            let mut content = String::new();
            for id in ids {
                let id = id.as_str().ok_or("Invalid Help article ID")?;
                let body = article(id).ok_or("Unknown Help article ID")?;
                content.push_str(&format!("Article {id}:\n{body}\n\n"));
            }
            messages.push(serde_json::json!({"role":"assistant", "content":payload}));
            messages.push(serde_json::json!({"role":"user", "content":content}));
        } else {
            let answer = parse_answer(&payload)?;
            return Ok(answer_json(&answer));
        }
    }
    Err("Help did not return an answer".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mocked_retrieval_and_unavailable_service() {
        let mut count = 0;
        let mut messages = vec![];
        let answer = grounded_reply(&mut messages, |wire| {
            count += 1;
            if count == 1 { Ok(r#"{"request_articles":["notifications"]}"#.into()) }
            else if count == 2 { assert!(wire.last().unwrap()["content"].as_str().unwrap().contains("500 entries")); Ok(r#"{"request_articles":["providers"]}"#.into()) }
            else { Ok(r#"{"answer":"Open the bell.","articles":["notifications"],"navigation":"notifications"}"#.into()) }
        }).unwrap();
        assert_eq!(count, 3);
        assert!(parse_answer(&answer).is_ok());
        assert!(grounded_reply(&mut vec![], |_| Ok(
            r#"{"request_articles":["unknown"]}"#.into()
        ))
        .is_err());
        assert!(grounded_reply(&mut vec![], |_| Ok(
            r#"{"request_articles":["help"]}"#.into()
        ))
        .unwrap_err()
        .contains("lookup limit"));
        assert!(
            grounded_reply(&mut vec![], |_| Err("koma-free unavailable".into()))
                .unwrap_err()
                .contains("unavailable")
        );
    }
    #[test]
    fn redaction_and_validation() {
        let clean = redact_context(
            &serde_json::json!({"attached":true,"platform":"linux","credentials":"secret","terminal":"output","activeView":"arbitrary-selector"}),
        );
        assert_eq!(
            clean,
            serde_json::json!({"attached":true,"platform":"linux"})
        );
        assert!(parse_answer(
            r#"{"answer":"Try Reference","articles":["providers"],"navigation":"connector"}"#
        )
        .is_ok());
        assert!(parse_answer(r#"{"answer":"Try","articles":["unknown"]}"#).is_err());
        assert!(parse_answer("text TOUR: git").is_err());
        // Fences / prose wrappers and unknown keys must still parse.
        assert!(parse_answer(
            "```json\n{\"answer\":\"Open Help → Reference.\",\"articles\":[\"help\"],\"extra\":true}\n```"
        )
        .is_ok());
        assert!(parse_answer(
            "Sure.\n{\"answer\":\"Use the activity bar.\",\"articles\":[\"help\"],\"navigation\":\"help\"}\n"
        )
        .is_ok());
        // GUI questions should not lead with TUI slash-command articles.
        let gui_hits = ranked("gimme help list in the GUI", 4);
        assert!(
            gui_hits.iter().any(|id| id == "help"),
            "expected help article, got {gui_hits:?}"
        );
        assert!(
            gui_hits.iter().all(|id| !id.starts_with("tui-")),
            "TUI articles leaked into GUI ranking: {gui_hits:?}"
        );
    }
    #[test]
    fn all_topics_and_targets_exist() {
        let m = manifest();
        assert!(m.is_ok(), "{}", m.as_ref().err().cloned().unwrap_or_default());
        if let Ok(m) = m {
            for a in &m.articles {
                assert!(article(&a.id).is_some());
                assert!(m.navigation.contains(&a.navigation));
                for id in &a.workflows {
                    assert!(m.workflows.contains(id));
                }
            }
        }
        assert!(ranked("OAuth model login", 3).contains(&"providers".to_string()));
        assert!(ranked("notification history", 3).contains(&"notifications".to_string()));
    }
}
