//! Project self-awareness (Phase 2): summarise the project's docs once per
//! session so the agent knows what it's working on without burning a full chat
//! round on it.
//!
//! On startup (and after `/compact`) the runtime reads the depth-1 project
//! docs (AGENT.md / AGENTS.md / README.md / CLAUDE.md) and asks a cheap
//! secondary model — via [`OpenRouterClient::complete_with`] — for a few-
//! sentence summary. That summary is stashed in `AppStateRest::awareness_summary`
//! and appended to the first System message on every request (see
//! `runtime::stream::start_stream_task`), so it survives compaction the same way
//! the top-level file listing does.
//!
//! Everything here is best-effort: a disabled flag, missing docs, no API key, or
//! a failed network call all degrade to `None`. The summary is never persisted —
//! it is recomputed per session so it always reflects the current docs.

use std::path::Path;

use crate::app::resolve::{same_route, Resolved};
use crate::dto::chat::{ChatMessage, Role};
use crate::model::settings::Settings;
use crate::service::openrouter::OpenRouterClient;

/// Project doc filenames probed at depth 1 of the workspace, in priority order.
/// Case-sensitive (matched verbatim) — the same names the rest of the app uses.
const DOC_FILES: &[&str] = &["AGENT.md", "AGENTS.md", "README.md", "CLAUDE.md"];

/// Max characters kept from any single doc file.
const PER_FILE_CAP: usize = 6000;

/// Max characters of combined doc corpus sent to the model.
const CORPUS_CAP: usize = 16000;

/// System instruction for the summary call. Kept terse and concrete so a small
/// model stays on task and returns prose the agent can use directly.
const SUMMARY_SYSTEM: &str = "You summarize a software project for an AI coding assistant. In 4-6 sentences, state what the project is, its stack, structure, and any conventions the agent must follow. Be concrete. No preamble.";

/// Take at most `cap` chars from `s` (char-boundary safe).
fn cap_chars(s: &str, cap: usize) -> String {
    s.chars().take(cap).collect()
}

#[derive(Debug, Default, Clone)]
pub(crate) struct SummaryResult {
    pub summary: Option<String>,
    pub status: Option<String>,
}

/// Identity travels with both startup and recompute results.
#[derive(Debug, Clone)]
pub(crate) struct AwarenessResult {
    pub session_id: String,
    pub workspace: std::path::PathBuf,
    pub generation: u64,
    pub result: SummaryResult,
}

impl AwarenessResult {
    pub(crate) fn apply(self, rt: &mut crate::app::state::SessionRuntime) -> bool {
        if rt.id != self.session_id
            || rt.awareness_generation != self.generation
            || rt.effective_cwd() != self.workspace
        {
            return false;
        }
        if let Some(status) = self.result.status.as_ref() {
            if rt.awareness_status.as_ref() != Some(status) {
                // Reuse the existing status/toast UI; suppress identical repeated notices.
                rt.set_toast_info(status.clone());
            }
        }
        if let crate::app::mode::Mode::Loading(ls) = &mut rt.mode {
            ls.awareness = crate::app::mode::WarmStatus::Done(
                if self.result.summary.is_some() {
                    "ready"
                } else {
                    "unavailable / no docs"
                }
                .into(),
            );
        }
        rt.awareness_status = self.result.status;
        rt.awareness_summary = self.result.summary;
        true
    }
}

/// Empty answers, timeouts and errors may try Main once. Missing docs never call a model.
pub(crate) async fn summarize_with_fallback(
    client: &OpenRouterClient,
    settings: &Settings,
    workdir: &Path,
    primary: Option<&Resolved>,
    main: Option<&Resolved>,
) -> SummaryResult {
    if !settings.awareness_enabled {
        return SummaryResult::default();
    }
    let mut corpus = String::new();
    for name in DOC_FILES {
        if corpus.chars().count() >= CORPUS_CAP {
            break;
        }
        let Ok(raw) = std::fs::read_to_string(workdir.join(name)) else {
            continue;
        };
        let body = cap_chars(raw.trim(), PER_FILE_CAP);
        if body.trim().is_empty() {
            continue;
        }
        let section = format!("## {name}\n{body}\n\n");
        let remaining = CORPUS_CAP - corpus.chars().count();
        corpus.push_str(&cap_chars(&section, remaining));
    }
    if corpus.trim().is_empty() {
        return SummaryResult::default();
    }
    let messages = vec![
        ChatMessage::new(Role::System, SUMMARY_SYSTEM),
        ChatMessage::new(Role::User, corpus),
    ];
    let fallback = main.filter(|m| primary.is_none_or(|p| !same_route(p, m)));
    let mut failures = Vec::new();
    if primary.is_none() {
        failures.push("primary configuration unavailable".into());
    }
    for (is_fallback, route) in [(false, primary), (true, fallback)] {
        let Some(route) = route else {
            continue;
        };
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(29),
            client.complete_with(
                route.conn(),
                &route.model_id,
                route.provider(),
                messages.clone(),
                false,
            ),
        )
        .await;
        let category = match response {
            Ok(Ok(text)) => {
                if let Some(summary) = crate::model::conversation::clean_compaction_summary(&text) {
                    return SummaryResult {
                        summary: Some(summary),
                        status: is_fallback.then(|| {
                            format!(
                                "Awareness: {}; using Main {}",
                                failures.join("; "),
                                route.model_id
                            )
                        }),
                    };
                }
                "empty"
            }
            Ok(Err(_)) => "request failed",
            Err(_) => "timeout",
        };
        failures.push(format!(
            "{} {}: {category}",
            if is_fallback { "fallback" } else { "primary" },
            route.model_id
        ));
    }
    SummaryResult {
        summary: None,
        status: Some(format!("Awareness unavailable: {}", failures.join("; "))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::SessionRuntime;

    fn result(rt: &SessionRuntime, generation: u64, summary: &str) -> AwarenessResult {
        AwarenessResult {
            session_id: rt.id.clone(),
            workspace: rt.effective_cwd(),
            generation,
            result: SummaryResult {
                summary: Some(summary.into()),
                status: None,
            },
        }
    }

    #[test]
    fn reverse_delivery_cannot_overwrite_newer_generation() {
        let mut rt = SessionRuntime::new();
        rt.awareness_generation = 2;
        let old = result(&rt, 1, "old");
        let new = result(&rt, 2, "new");
        assert!(new.apply(&mut rt));
        assert!(!old.apply(&mut rt));
        assert_eq!(rt.awareness_summary.as_deref(), Some("new"));
    }

    #[test]
    fn workspace_or_session_change_discards_result() {
        let mut rt = SessionRuntime::new();
        let old = result(&rt, 0, "old workspace");
        rt.active_cwd = Some(std::path::PathBuf::from("/different-workspace"));
        assert!(!old.apply(&mut rt));
        let mut wrong_session = result(&rt, 0, "other session");
        wrong_session.session_id = "other".into();
        assert!(!wrong_session.apply(&mut rt));
        assert!(rt.awareness_summary.is_none());
    }
}

#[cfg(test)]
mod provider_tests {
    use super::*;
    use crate::app::harness::regression_test::{fixture, response};
    use crate::app::resolve::resolve_role_dispatch;
    use crate::model::app_config::ModelRole;

    #[tokio::test]
    async fn empty_primary_tries_main_once_and_reports_the_route() {
        let path = std::env::temp_dir().join(format!("koma-aware-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("README.md"), "A small project.").unwrap();
        let (mut config, requests) = fixture(
            vec![
                (response(serde_json::Value::Null, "stop"), 0),
                (response(serde_json::json!("Valid summary."), "stop"), 0),
            ],
            true,
        )
        .await;
        config.models[0].roles = vec![ModelRole::Awareness];
        let settings = Settings {
            awareness_enabled: true,
            ..Default::default()
        };
        let primary = resolve_role_dispatch(&config, &settings, ModelRole::Awareness);
        let main = resolve_role_dispatch(&config, &settings, ModelRole::Main);
        let result = summarize_with_fallback(
            &OpenRouterClient::new(),
            &settings,
            &path,
            primary.as_ref(),
            main.as_ref(),
        )
        .await;
        assert_eq!(result.summary.as_deref(), Some("Valid summary."));
        assert!(result.status.unwrap().contains("using Main fallback"));
        assert_eq!(requests.await.unwrap().len(), 2);
        std::fs::remove_dir_all(path).unwrap();
    }
}
