//! One-shot model re-rank for `skill_find`.
//!
//! The word match gives a short list. When no candidate clearly wins, the Main model reads the
//! request and the candidates' one-liners and says which fit best. It is one small call; any
//! failure, empty answer or timeout returns `None` and the word-match order stands.

use std::time::Duration;

use crate::app::resolve::resolve_role_dispatch;
use crate::dto::chat::{ChatMessage, Role};
use crate::model::app_config::{AppConfig, ModelRole};
use crate::model::settings::Settings;
use crate::service::openrouter::OpenRouterClient;
use crate::tool::SkillRank;

const TIMEOUT: Duration = Duration::from_secs(8);

const SYSTEM: &str = "You choose which saved skills fit a request. You get the request and a numbered list \
of candidate skills, each `name: one-line description`. Answer with the names of the skills that fit, best \
first, one per line, at most five. Use only names from the list. If none fit, answer NONE. No other text.";

pub fn prompt(request: &str, candidates: &[(String, String)]) -> Vec<ChatMessage> {
    let list: String = candidates.iter().enumerate().map(|(i, (n, d))| format!("{}. {n}: {d}\n", i + 1)).collect();
    vec![ChatMessage::new(Role::System, SYSTEM), ChatMessage::new(Role::User, format!("Request: {request}\n\nCandidates:\n{list}"))]
}

/// The candidate names a reply mentions, in the order the reply gives them.
pub fn parse(reply: &str, candidates: &[(String, String)]) -> Option<Vec<String>> {
    let mut found: Vec<(usize, String)> = Vec::new();
    for (name, _) in candidates {
        let at = reply.to_lowercase().find(&name.to_lowercase());
        if let Some(at) = at {
            found.push((at, name.clone()));
        }
    }
    found.sort_by_key(|(at, _)| *at);
    (!found.is_empty()).then(|| found.into_iter().map(|(_, n)| n).collect())
}

/// A re-rank that asks the Main route. The call runs on its own thread with its own small runtime,
/// so it works the same from a plain thread (the chat) and from inside an async runtime (the room).
pub fn ranker(config: AppConfig, settings: Settings) -> SkillRank {
    std::sync::Arc::new(move |request: &str, candidates: &[(String, String)]| {
        let route = resolve_role_dispatch(&config, &settings, ModelRole::Main)?;
        let messages = prompt(request, candidates);
        let worker = std::thread::Builder::new().name("skill-rank".into()).spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
            rt.block_on(async {
                let client = OpenRouterClient::new();
                tokio::time::timeout(TIMEOUT, client.complete(route.conn(), &route.model_id, route.provider(), messages)).await.ok()?.ok()
            })
        });
        let reply = worker.ok()?.join().ok()??;
        parse(&reply, candidates)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cands() -> Vec<(String, String)> {
        vec![("jwt-review".into(), "tokens".into()), ("sql-hunt".into(), "queries".into()), ("xss-scan".into(), "scripts".into())]
    }

    #[test]
    fn the_reply_order_becomes_the_rank() {
        assert_eq!(parse("sql-hunt\njwt-review", &cands()), Some(vec!["sql-hunt".into(), "jwt-review".into()]));
        assert_eq!(parse("1. XSS-SCAN", &cands()), Some(vec!["xss-scan".into()]));
    }

    #[test]
    fn no_match_or_none_leaves_the_word_order() {
        assert_eq!(parse("NONE", &cands()), None);
        assert_eq!(parse("", &cands()), None);
        assert_eq!(parse("something unrelated", &cands()), None);
    }

    #[test]
    fn the_prompt_carries_the_request_and_every_candidate() {
        let m = prompt("check tokens", &cands());
        assert_eq!(m.len(), 2);
        assert!(m[1].content.contains("check tokens") && m[1].content.contains("2. sql-hunt: queries"));
    }
}
