//! Unit tests for the agent_def module.

use std::collections::HashMap;

use super::def::{AgentDef, AgentSource};
use super::parse::{parse_agent, split_frontmatter, validate_agent_name};
use super::registry::{builtin_agents, merge_extension_sub_agents};

#[test]
fn parses_full_frontmatter_and_body() {
    let content = "---\n\
description: Do the thing\n\
model: openai/gpt-oss-20b\n\
provider: groq\n\
tools: [read, grep, edit]\n\
steps: 12\n\
effort: high\n\
temperature: 0.4\n\
color: cyan\n\
hidden: true\n\
---\n\
You are a focused subagent.\nReport tersely.";

    let a = parse_agent("MyAgent", content, AgentSource::Global).unwrap();
    assert_eq!(a.name, "myagent"); // lowercased
    assert_eq!(a.description, "Do the thing");
    assert_eq!(a.model.as_deref(), Some("openai/gpt-oss-20b"));
    assert_eq!(a.provider.as_deref(), Some("groq"));
    assert_eq!(a.tools, vec!["read", "grep", "edit"]);
    assert_eq!(a.steps, Some(12));
    assert_eq!(a.effort.as_deref(), Some("high"));
    assert_eq!(a.temperature, Some(0.4));
    assert_eq!(a.color.as_deref(), Some("cyan"));
    assert!(a.hidden);
    assert_eq!(a.source, AgentSource::Global);
    assert_eq!(
        a.prompt,
        "You are a focused subagent.\nReport tersely."
    );
}

#[test]
fn body_only_file_has_defaults_and_prompt() {
    // No frontmatter fence at all: the whole content is the body. But the
    // loader requires a description, so a pure body-only file is rejected.
    // Here we assert the SPLIT + default behavior via a frontmatter that
    // only carries the required description, leaving everything else default.
    let content = "---\ndescription: Minimal\n---\nThis is the system prompt body.";
    let a = parse_agent("minimal", content, AgentSource::Session).unwrap();
    assert_eq!(a.prompt, "This is the system prompt body.");
    assert!(a.model.is_none());
    assert!(a.provider.is_none());
    assert!(a.tools.is_empty());
    assert!(a.steps.is_none());
    assert!(!a.hidden);
    assert!(!a.disable);
    // Empty tools -> effective tools fall back to the read-only default.
    assert_eq!(
        a.effective_tools(),
        vec!["read", "grep", "glob", "dir_list"]
    );
}

#[test]
fn pure_body_no_frontmatter_splits_as_all_body() {
    // split_frontmatter alone: a file with no opening fence is all body.
    let (fm, body) = split_frontmatter("just a body, no fence\nsecond line").unwrap();
    assert!(fm.is_none());
    assert_eq!(body, "just a body, no fence\nsecond line");
}

#[test]
fn malformed_frontmatter_is_an_error_not_a_panic() {
    // Unclosed fence.
    let unclosed = "---\ndescription: oops\nno closing fence here";
    assert!(parse_agent("x", unclosed, AgentSource::Global).is_err());

    // Invalid YAML inside a closed fence.
    let bad_yaml = "---\ndescription: [unterminated\n---\nbody";
    assert!(parse_agent("x", bad_yaml, AgentSource::Global).is_err());

    // Missing required description.
    let no_desc = "---\nmodel: foo/bar\n---\nbody";
    assert!(parse_agent("x", no_desc, AgentSource::Global).is_err());
}

#[test]
fn tools_allow_list_parsed_and_task_stripped() {
    let content =
        "---\ndescription: d\ntools: [read, task, bash]\n---\nbody";
    let a = parse_agent("t", content, AgentSource::Global).unwrap();
    // Raw parse keeps what was written...
    assert_eq!(a.tools, vec!["read", "task", "bash"]);
    // ...but the effective allow-list strips the "task" recursion guard.
    assert_eq!(a.effective_tools(), vec!["read", "bash"]);
}

#[test]
fn builtins_present_with_expected_shape() {
    let builtins = builtin_agents();
    assert_eq!(builtins.len(), 2);

    let explore = builtins.iter().find(|a| a.name == "explore").unwrap();
    assert_eq!(explore.source, AgentSource::Builtin);
    assert!(explore.model.is_none()); // inherit
    assert_eq!(explore.steps, Some(80));
    assert_eq!(explore.tools, vec!["read", "grep", "glob", "dir_list"]);
    assert!(!explore.prompt.trim().is_empty());

    let general = builtins.iter().find(|a| a.name == "general").unwrap();
    assert_eq!(general.steps, Some(25));
    assert!(general.tools.contains(&"bash".to_string()));
    // The general agent must never carry the task tool.
    assert!(!general.tools.contains(&"task".to_string()));
}

#[test]
fn precedence_session_over_global_over_builtin() {
    let mut agents: HashMap<String, AgentDef> = HashMap::new();

    // Built-in "explore".
    for a in builtin_agents() {
        agents.insert(a.name.clone(), a);
    }
    assert_eq!(agents["explore"].source, AgentSource::Builtin);

    // Global overrides built-in.
    let global = parse_agent(
        "explore",
        "---\ndescription: global explore\n---\nglobal body",
        AgentSource::Global,
    )
    .unwrap();
    agents.insert(global.name.clone(), global);
    assert_eq!(agents["explore"].source, AgentSource::Global);
    assert_eq!(agents["explore"].description, "global explore");

    // Session overrides global.
    let session = parse_agent(
        "explore",
        "---\ndescription: session explore\n---\nsession body",
        AgentSource::Session,
    )
    .unwrap();
    agents.insert(session.name.clone(), session);
    assert_eq!(agents["explore"].source, AgentSource::Session);
    assert_eq!(agents["explore"].description, "session explore");
}

#[test]
fn disable_removes_agent_from_registry() {
    let mut agents: HashMap<String, AgentDef> = HashMap::new();
    for a in builtin_agents() {
        agents.insert(a.name.clone(), a);
    }
    // A disabling override of a built-in.
    let off = parse_agent(
        "general",
        "---\ndescription: gone\ndisable: true\n---\nbody",
        AgentSource::Global,
    )
    .unwrap();
    agents.insert(off.name.clone(), off);

    // Same retain step the loader applies.
    agents.retain(|_, a| !a.disable);

    assert!(!agents.contains_key("general"));
    assert!(agents.contains_key("explore"));
}

#[test]
fn name_sanitization_rules() {
    // Uppercasing -> lowercase.
    assert_eq!(validate_agent_name("PINEAPPLE").unwrap(), "pineapple");
    assert_eq!(validate_agent_name("My-Agent").unwrap(), "my-agent");

    // Path traversal rejected.
    assert!(validate_agent_name("../x").is_err());
    assert!(validate_agent_name("..").is_err());
    assert!(validate_agent_name("a/b").is_err());
    assert!(validate_agent_name("a\\b").is_err());

    // Empty rejected.
    assert!(validate_agent_name("").is_err());

    // Leading/trailing dash rejected.
    assert!(validate_agent_name("-x").is_err());
    assert!(validate_agent_name("x-").is_err());

    // Illegal chars rejected.
    assert!(validate_agent_name("a b").is_err());
    assert!(validate_agent_name("a.b").is_err());
}

/// Wave B: an installed + enabled extension's `contributes.sub_agents` appears
/// in the merged registry tagged `AgentSource::Extension`, and disappears again
/// both on `enabled = false` and on removing the config entry entirely (the
/// uninstall shape) — all against a temp `ext_root`, never the real `~/.koma`.
#[test]
fn extension_sub_agent_appears_and_disappears_with_installed_extensions() {
    use crate::model::app_config::{AppConfig, InstalledExtension};

    let tmp = std::env::temp_dir().join(format!(
        "koma-agentdef-ext-test-{}",
        uuid::Uuid::new_v4()
    ));
    let ext_id = "run.koma.example.subagent-test";
    let ext_dir = tmp.join(ext_id);
    std::fs::create_dir_all(&ext_dir).expect("create ext dir");
    let manifest = serde_json::json!({
        "schema": "koma-extension/v0",
        "id": ext_id,
        "name": "Sub Agent Test",
        "version": "0.0.1",
        "tier": "free",
        "kind": "daemon",
        "runtime": { "exec": "bin/tool", "args": [] },
        "contributes": {
            "sub_agents": [
                { "name": "Reviewer", "description": "Reviews code for style issues." }
            ]
        },
        "requires": []
    });
    std::fs::write(ext_dir.join("manifest.json"), manifest.to_string())
        .expect("write manifest");

    let mut config = AppConfig::default();
    config.installed_extensions.push(InstalledExtension {
        id: ext_id.to_string(),
        version: "0.0.1".to_string(),
        tier: "free".to_string(),
        granted: vec![],
        enabled: true,
        kind: "daemon".to_string(),
        exec: "bin/tool".to_string(),
    });

    // Enabled -> the sub-agent appears, name lowercased, tagged Extension.
    let mut agents: HashMap<String, AgentDef> = HashMap::new();
    merge_extension_sub_agents(&config, &tmp, &mut agents);
    assert!(agents.contains_key("reviewer"), "sub-agent should appear when enabled");
    assert_eq!(agents["reviewer"].source, AgentSource::Extension);
    assert_eq!(agents["reviewer"].description, "Reviews code for style issues.");
    assert_eq!(agents["reviewer"].conditions, "Reviews code for style issues.");

    // Disabled (still installed) -> gone.
    config.installed_extensions[0].enabled = false;
    let mut agents_disabled: HashMap<String, AgentDef> = HashMap::new();
    merge_extension_sub_agents(&config, &tmp, &mut agents_disabled);
    assert!(
        !agents_disabled.contains_key("reviewer"),
        "sub-agent must disappear once its extension is disabled"
    );

    // Uninstalled (config entry removed entirely) -> also gone.
    config.remove_extension_by_id(ext_id);
    let mut agents_uninstalled: HashMap<String, AgentDef> = HashMap::new();
    merge_extension_sub_agents(&config, &tmp, &mut agents_uninstalled);
    assert!(
        !agents_uninstalled.contains_key("reviewer"),
        "sub-agent must disappear once its extension is uninstalled"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn to_markdown_round_trips_set_fields_only() {
    let a = AgentDef {
        name: "rt".to_string(),
        description: "round trip".to_string(),
        model: Some("openai/gpt-oss-20b".to_string()),
        tools: vec!["read".to_string(), "edit".to_string()],
        steps: Some(7),
        prompt: "Body here.".to_string(),
        ..Default::default()
    };
    let md = a.to_markdown();
    assert!(md.starts_with("---\n"));
    assert!(md.contains("description: round trip"));
    assert!(md.contains("model: openai/gpt-oss-20b"));
    assert!(md.ends_with("Body here."));
    // provider/effort/etc. are unset -> not emitted.
    assert!(!md.contains("provider:"));
    assert!(!md.contains("effort:"));

    // Re-parse and confirm the set fields survive.
    let b = parse_agent("rt", &md, AgentSource::Session).unwrap();
    assert_eq!(b.description, "round trip");
    assert_eq!(b.model.as_deref(), Some("openai/gpt-oss-20b"));
    assert_eq!(b.tools, vec!["read", "edit"]);
    assert_eq!(b.steps, Some(7));
    assert_eq!(b.prompt, "Body here.");
}
