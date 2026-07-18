//! W13 additional regression suite for `protocol.rs` — PURE ADDITION alongside the existing
//! inline `mod tests` in that file (never touched here). Focuses on gaps the inline suite
//! doesn't already cover: an EXHAUSTIVE Grant wire-string canary (every variant, compile-time
//! guarded against a silently-unmapped future addition), frame roundtrips under optional-field
//! permutations + field-order robustness, manifest forward-compat against unknown future keys
//! at every nesting level, and `SubAgentDef`/`OAuthProviderDef` edge values (empty strings,
//! unicode). Cases already proven by the existing `tests` module (old-style `Contributes`
//! back-compat, `KomaMsg::Event`/`ExtMsg::Notify` roundtrips, unknown-tag rejection, the
//! 6-of-8-variant `Grant` wire spot-check, minimal/full `OAuthProviderDef` roundtrips) are
//! deliberately NOT re-derived here.

use super::*;
use serde_json::json;

/// EXHAUSTIVE Grant wire-string canary: a `match` with **no wildcard arm**, so adding a new
/// `Grant` variant fails to COMPILE this test until it is added here too. For every variant:
/// the wire string is non-empty, every variant's wire string is DISTINCT from every other's,
/// and `parse(wire(g)) == g` round-trips through serde exactly.
#[test]
fn grant_wire_strings_exhaustive_canary() {
    let all = [
        Grant::AgentsRead,
        Grant::AgentsOrchestrate,
        Grant::SessionsManage,
        Grant::ChatPrompt,
        Grant::ModelsInvoke,
        Grant::ContextPublish,
        Grant::OauthContribute,
        Grant::ModelsContribute,
    ];

    let wire_of = |g: Grant| -> &'static str {
        // Exhaustive match, no `_` arm: a new variant not added here breaks the build.
        match g {
            Grant::AgentsRead => "agents:read",
            Grant::AgentsOrchestrate => "agents:orchestrate",
            Grant::SessionsManage => "sessions:manage",
            Grant::ChatPrompt => "chat:prompt",
            Grant::ModelsInvoke => "models:invoke",
            Grant::ContextPublish => "context:publish",
            Grant::OauthContribute => "oauth:contribute",
            Grant::ModelsContribute => "models:contribute",
        }
    };

    let mut seen_wires: Vec<&'static str> = Vec::new();
    for g in all {
        let expected = wire_of(g);
        assert!(
            !expected.is_empty(),
            "{g:?} must have a non-empty wire string"
        );

        // The serde-emitted wire string matches the documented mapping exactly.
        let serialized = serde_json::to_string(&g).expect("serializes");
        assert_eq!(
            serialized,
            format!("\"{expected}\""),
            "{g:?} serde tag must match wire_of"
        );

        // Distinct from every other variant's wire string so far.
        assert!(
            !seen_wires.contains(&expected),
            "{g:?}'s wire string {expected:?} collides with an earlier variant"
        );
        seen_wires.push(expected);

        // parse(wire(g)) == g.
        let back: Grant = serde_json::from_str(&serialized).expect("deserializes");
        assert_eq!(back, g, "{g:?} must round-trip through its own wire string");
    }
    assert_eq!(
        seen_wires.len(),
        all.len(),
        "every variant must have contributed a distinct wire"
    );
}

/// `ExtMsg::Call` round-trips with EVERY combination of its fields present/absent-equivalent
/// (params as an object, an empty object, and a non-object scalar — `params` has no `#[serde
/// (default)]` so it is always required on the wire, but its VALUE shape must be permutable),
/// and is robust to FIELD ORDER — the tag `"t"` is not necessarily first on the wire.
#[test]
fn ext_msg_call_roundtrips_field_order_and_param_shapes() {
    // Canonical field order (as `to_value` emits it).
    let msg = ExtMsg::Call {
        id: 7,
        method: "agents.spawn".to_string(),
        params: json!({ "task": "x" }),
    };
    let wire = serde_json::to_value(&msg).expect("serializes");
    let back: ExtMsg = serde_json::from_value(wire.clone()).expect("deserializes");
    match back {
        ExtMsg::Call { id, method, params } => {
            assert_eq!(id, 7);
            assert_eq!(method, "agents.spawn");
            assert_eq!(params, json!({ "task": "x" }));
        }
        other => panic!("expected ExtMsg::Call, got {other:?}"),
    }

    // Field-order robustness: hand-roll the SAME frame with fields in reverse + tag LAST.
    let reordered = json!({
        "params": { "task": "x" },
        "method": "agents.spawn",
        "id": 7,
        "t": "call",
    });
    let back2: ExtMsg = serde_json::from_value(reordered).expect("field order must not matter");
    match back2 {
        ExtMsg::Call { id, method, params } => {
            assert_eq!(id, 7);
            assert_eq!(method, "agents.spawn");
            assert_eq!(params, json!({ "task": "x" }));
        }
        other => panic!("expected ExtMsg::Call, got {other:?}"),
    }

    // params as an empty object and as a non-object scalar both round-trip (params is untyped
    // `serde_json::Value` — the wire protocol places no shape constraint on it here).
    for shape in [
        json!({}),
        json!("bare-string"),
        json!(42),
        json!(null),
        json!([1, 2, 3]),
    ] {
        let m = ExtMsg::Call {
            id: 1,
            method: "x.y".to_string(),
            params: shape.clone(),
        };
        let v = serde_json::to_value(&m).expect("serializes");
        let back: ExtMsg = serde_json::from_value(v).expect("deserializes");
        match back {
            ExtMsg::Call { params, .. } => assert_eq!(params, shape),
            other => panic!("expected ExtMsg::Call, got {other:?}"),
        }
    }
}

/// `KomaMsg::Welcome` round-trips with an EMPTY `granted` list and with a MULTI-element one,
/// and is robust to field order (the tag not first, `granted` before `koma_version`).
#[test]
fn koma_msg_welcome_roundtrips_optional_permutations_and_field_order() {
    // Empty granted set (a freshly-installed extension with no grants echoed yet).
    let empty = KomaMsg::Welcome {
        protocol: PROTOCOL_VERSION.to_string(),
        koma_version: "0.2.28".to_string(),
        granted: Vec::new(),
    };
    let wire = serde_json::to_value(&empty).expect("serializes");
    assert_eq!(wire["granted"], json!([]));
    let back: KomaMsg = serde_json::from_value(wire).expect("deserializes");
    match back {
        KomaMsg::Welcome { granted, .. } => assert!(granted.is_empty()),
        other => panic!("expected KomaMsg::Welcome, got {other:?}"),
    }

    // Multi-grant set, reordered wire (tag last, granted before koma_version/protocol).
    let reordered = json!({
        "granted": ["agents:read", "chat:prompt"],
        "koma_version": "0.2.28",
        "protocol": PROTOCOL_VERSION,
        "t": "welcome",
    });
    let back2: KomaMsg = serde_json::from_value(reordered).expect("field order must not matter");
    match back2 {
        KomaMsg::Welcome {
            protocol,
            koma_version,
            granted,
        } => {
            assert_eq!(protocol, PROTOCOL_VERSION);
            assert_eq!(koma_version, "0.2.28");
            assert_eq!(granted, vec![Grant::AgentsRead, Grant::ChatPrompt]);
        }
        other => panic!("expected KomaMsg::Welcome, got {other:?}"),
    }
}

/// A full [`ExtensionManifest`] with UNKNOWN keys injected at EVERY nesting level (top-level,
/// inside `runtime`, inside `contributes`, inside a `contributes.sub_agents[]` element, and
/// inside a `requires[]` element is not applicable since `Grant` is a closed enum — instead an
/// unknown key sits beside a known `requires` entry) still parses cleanly, proving forward-compat:
/// a FUTURE koma version's manifest fields never break an OLDER manifest reader.
#[test]
fn manifest_forward_compat_unknown_keys_at_every_level_parses() {
    let raw = json!({
        "schema": MANIFEST_SCHEMA,
        "id": "run.koma.example.forward-compat",
        "name": "Forward Compat",
        "version": "1.0.0",
        "tier": "free",
        "kind": "daemon",
        "runtime": {
            "exec": "bin/daemon",
            "args": [],
            "future_runtime_key": { "nested": true },
        },
        "contributes": {
            "sub_agents": [
                {
                    "name": "planner",
                    "description": "plans things",
                    "future_subagent_key": 42,
                }
            ],
            "future_contributes_key": ["a", "b"],
        },
        "requires": ["agents:read"],
        "future_top_level_key": { "anything": [1, 2, 3] },
    });
    let manifest: ExtensionManifest =
        serde_json::from_value(raw).expect("unknown keys at every level must not fail parsing");
    assert_eq!(manifest.id, "run.koma.example.forward-compat");
    assert_eq!(manifest.contributes.sub_agents.len(), 1);
    assert_eq!(manifest.contributes.sub_agents[0].name, "planner");
    assert_eq!(manifest.requires, vec![Grant::AgentsRead]);
}

/// `SubAgentDef` edge values: empty (but present) optional strings are preserved AS EMPTY
/// STRINGS (not coerced to `None` — only JSON `null`/absence maps to `None` for an
/// `Option<String>`), and unicode in `name`/`description`/`prompt` round-trips byte-for-byte.
#[test]
fn subagent_def_edge_values_empty_strings_and_unicode() {
    let def = SubAgentDef {
        name: "翻訳エージェント 🐉".to_string(),
        description: "Ünïcödé déscription with emoji 🚀✨".to_string(),
        prompt: Some(String::new()),
        model: Some(String::new()),
        effort: Some("".to_string()),
        tools: Vec::new(),
    };
    let wire = serde_json::to_value(&def).expect("serializes");
    // Present-but-empty optional strings serialize as `""`, not omitted / null.
    assert_eq!(wire["prompt"], json!(""));
    assert_eq!(wire["model"], json!(""));
    assert_eq!(wire["effort"], json!(""));

    let back: SubAgentDef = serde_json::from_value(wire).expect("deserializes");
    assert_eq!(back.name, "翻訳エージェント 🐉");
    assert_eq!(back.description, "Ünïcödé déscription with emoji 🚀✨");
    assert_eq!(back.prompt.as_deref(), Some(""));
    assert_eq!(back.model.as_deref(), Some(""));
    assert_eq!(back.effort.as_deref(), Some(""));
}

/// `OAuthProviderDef` edge values: an empty `id`/`name`/`method` still round-trips (the wire
/// type itself imposes no non-empty constraint — validation, if any, is a koma-host concern),
/// and unicode in `name` round-trips byte-for-byte.
#[test]
fn oauth_provider_def_edge_values_empty_and_unicode() {
    let def = OAuthProviderDef {
        id: String::new(),
        name: "アカウント ログイン 🔐".to_string(),
        method: String::new(),
        chat_endpoint: None,
        api_type: None,
        refresh: None,
    };
    let wire = serde_json::to_value(&def).expect("serializes");
    assert_eq!(wire["id"], json!(""));
    assert_eq!(wire["method"], json!(""));
    let back: OAuthProviderDef = serde_json::from_value(wire).expect("deserializes");
    assert_eq!(back.id, "");
    assert_eq!(back.name, "アカウント ログイン 🔐");
    assert_eq!(back.method, "");
}

/// A full [`ExtensionManifest`] declaring `contributes.tui_screens` round-trips: the
/// `TuiScreenDef` list survives serialize→parse byte-for-shape (both fields, in order,
/// multi-element), and — mirroring `oauth_providers`' omitted-when-empty behavior — a
/// manifest with NO `tui_screens` entries serializes with the key OMITTED entirely (not an
/// empty array on the wire), so an old reader predating this field sees byte-identical JSON.
#[test]
fn manifest_with_tui_screens_roundtrips() {
    let raw = json!({
        "schema": MANIFEST_SCHEMA,
        "id": "run.koma.example.tui-demo-daemon",
        "name": "TUI Demo",
        "version": "0.0.0",
        "tier": "free",
        "kind": "daemon",
        "runtime": { "exec": "tui-demo-daemon", "args": [] },
        "contributes": {
            "tui_screens": [
                { "id": "demo", "title": "TUI Demo" },
                { "id": "second", "title": "Second Screen" }
            ]
        },
        "requires": [],
    });
    let manifest: ExtensionManifest =
        serde_json::from_value(raw).expect("a manifest with tui_screens parses");
    assert_eq!(manifest.contributes.tui_screens.len(), 2);
    assert_eq!(manifest.contributes.tui_screens[0].id, "demo");
    assert_eq!(manifest.contributes.tui_screens[0].title, "TUI Demo");
    assert_eq!(manifest.contributes.tui_screens[1].id, "second");
    assert_eq!(manifest.contributes.tui_screens[1].title, "Second Screen");

    // Round-trip back through serde_json::Value and re-parse.
    let wire = serde_json::to_value(&manifest).expect("serializes");
    assert_eq!(wire["contributes"]["tui_screens"][0]["id"], "demo");
    assert_eq!(wire["contributes"]["tui_screens"][1]["title"], "Second Screen");
    let back: ExtensionManifest =
        serde_json::from_value(wire).expect("re-parses after round-trip");
    assert_eq!(back.contributes.tui_screens.len(), 2);
    assert_eq!(back.contributes.tui_screens[1].id, "second");

    // A manifest with an empty `tui_screens` (the default) omits the key entirely on the wire.
    let empty = ExtensionManifest {
        schema: MANIFEST_SCHEMA.to_string(),
        id: "run.koma.example.no-screens".to_string(),
        name: "No Screens".to_string(),
        version: "0.0.0".to_string(),
        description: String::new(),
        tier: Tier::Free,
        kind: ExtensionKind::Daemon,
        runtime: Runtime { exec: "bin/x".to_string(), args: Vec::new() },
        contributes: Contributes::default(),
        requires: Vec::new(),
        workspace_dir: None,
        mcp_servers: Vec::new(),
    };
    let empty_wire = serde_json::to_value(&empty).expect("serializes");
    assert_eq!(
        empty_wire["contributes"].get("tui_screens"),
        None,
        "empty tui_screens must be omitted, not serialized as []"
    );
}
