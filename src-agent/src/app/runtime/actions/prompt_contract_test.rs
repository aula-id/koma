#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::app::state::AgentMode;
use crate::model::conversation::Conversation;
use crate::model::session::Session;
use crate::model::settings::Settings;

fn mk_session(tag: &str) -> Session {
    let dir = std::env::temp_dir().join(format!(
        "koma-prompt-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    Session::new(
        format!("s-{tag}"),
        dir,
        "pwd".into(),
        Settings::default(),
        Conversation::from_messages(vec![]),
    )
}

/// Build the system prompt for a session in the given agent mode.
fn prompt_for_mode(mode: AgentMode) -> String {
    let mut sess = mk_session(&format!("prompt-{mode:?}"));
    match mode {
        AgentMode::Plan => {
            sess.plan_mode_hint = true;
            sess.sdlc_mode_hint = false;
        }
        AgentMode::Sdlc => {
            sess.plan_mode_hint = false;
            sess.sdlc_mode_hint = true;
        }
        _ => {
            sess.plan_mode_hint = false;
            sess.sdlc_mode_hint = false;
        }
    }
    sess.rebuild_system();
    // System prompt is the first message in conversation after rebuild.
    sess.conversation
        .messages()
        .first()
        .map(|m| m.content.clone())
        .unwrap_or_default()
}

/// Keywords that belong ONLY to SDLC and must not appear in Auto/Normal/Yolo/Plan prompts.
const SDLC_ONLY_KEYWORDS: &[&str] = &[
    "SDLC mode",
    "mission_ready",
    "mission_verify",
    "mission_prepare",
    "mission_integrate",
    "keeper",
    "worktree binding",
    "frozen target",
    "OPEN leaf",
    "SEALED",
    "Path ownership",
    "epic",
];

/// Section 4: Auto mode prompt has no SDLC rail/capsule/lifecycle/hierarchy language.
#[test]
fn auto_mode_prompt_no_sdlc_language() {
    let prompt = prompt_for_mode(AgentMode::Auto);
    for keyword in SDLC_ONLY_KEYWORDS {
        assert!(
            !prompt.contains(keyword),
            "Auto mode prompt must not contain SDLC keyword '{keyword}'"
        );
    }
}

/// Section 4: Normal mode prompt has no SDLC language.
#[test]
fn normal_mode_prompt_no_sdlc_language() {
    let prompt = prompt_for_mode(AgentMode::Normal);
    for keyword in SDLC_ONLY_KEYWORDS {
        assert!(
            !prompt.contains(keyword),
            "Normal mode prompt must not contain SDLC keyword '{keyword}'"
        );
    }
}

/// Section 4: Yolo mode prompt has no SDLC language.
#[test]
fn yolo_mode_prompt_no_sdlc_language() {
    let prompt = prompt_for_mode(AgentMode::Yolo);
    for keyword in SDLC_ONLY_KEYWORDS {
        assert!(
            !prompt.contains(keyword),
            "Yolo mode prompt must not contain SDLC keyword '{keyword}'"
        );
    }
}

/// Section 4: Plan mode prompt has no SDLC language.
#[test]
fn plan_mode_prompt_no_sdlc_language() {
    let prompt = prompt_for_mode(AgentMode::Plan);
    for keyword in SDLC_ONLY_KEYWORDS {
        assert!(
            !prompt.contains(keyword),
            "Plan mode prompt must not contain SDLC keyword '{keyword}'"
        );
    }
}

/// Section 4: SDLC mode prompt DOES contain SDLC phase/lifecycle instructions.
#[test]
fn sdlc_mode_prompt_has_sdlc_language() {
    let prompt = prompt_for_mode(AgentMode::Sdlc);
    assert!(
        prompt.contains("SDLC"),
        "SDLC mode prompt must contain SDLC instructions"
    );
    assert!(
        prompt.contains("mission_ready"),
        "SDLC mode prompt must reference mission_ready"
    );
}

/// Section 4: Auto mode prompt has no auto-checklist mandate.
#[test]
fn auto_mode_prompt_no_auto_checklist_mandate() {
    let prompt = prompt_for_mode(AgentMode::Auto);
    assert!(
        !prompt.contains("Starting a complex multi-step task to break it down"),
        "prompt must not contain the old auto-checklist mandate"
    );
}

/// Section 4: Plan mode prompt has Plan-specific guidance.
#[test]
fn plan_mode_prompt_has_plan_guidance() {
    let prompt = prompt_for_mode(AgentMode::Plan);
    assert!(
        prompt.contains("Plan mode"),
        "Plan mode prompt must contain Plan guidance"
    );
    assert!(
        prompt.contains("READ-ONLY"),
        "Plan mode prompt must stress read-only"
    );
    assert!(
        prompt.contains("plan_ready"),
        "Plan mode prompt must mention plan_ready"
    );
    assert!(
        prompt.contains("DO NOT call write"),
        "Plan mode prompt must explicitly forbid write/edit/bash"
    );
}
