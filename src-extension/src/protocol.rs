use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: &str = "v0";
pub const MANIFEST_SCHEMA: &str = "koma-extension/v0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub schema: String, // MANIFEST_SCHEMA
    pub id: String,     // reverse-DNS, e.g. "run.koma.example.echo-tool-daemon"
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub tier: Tier,
    pub kind: ExtensionKind,
    pub runtime: Runtime,
    #[serde(default)]
    pub contributes: Contributes,
    #[serde(default)]
    pub requires: Vec<Grant>,
    /// Optional dedicated state directory this extension owns, declared as a path
    /// string (typically `"~/.<ext-name>"`, e.g. `"~/.event-watcher"`). When present,
    /// koma validates it, CREATES it if missing, and injects it as an extra workspace
    /// root of every session so the agent's file tools + `bash` may read/write there
    /// (an extension's own sub-agents can persist state that survives a restart).
    ///
    /// The path must resolve STRICTLY under `$HOME` (`%USERPROFILE%` on Windows); koma
    /// rejects `$HOME` itself, its own `~/.koma` tree, the credential stores `~/.ssh` /
    /// `~/.aws` / `~/.gnupg` (and anything under them), and `~/.config` itself (its
    /// subdirectories are allowed). A path that fails validation is logged and skipped —
    /// it never blocks the extension from starting. Serde-default/optional so a manifest
    /// predating this field parses unchanged. See `docs/EXTENSIONS.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    /// Bundled stdio MCP servers this extension ships (e.g. a standalone MCP binary
    /// spawned alongside its own `runtime.exec` daemon, like the Workflow extension's
    /// `bin/workflow-mcp`) that koma should AUTO-REGISTER into the global MCP catalogue
    /// at install time. Without this, a bundled MCP server needs the user to hand-add
    /// an `McpServerEntry` through the MCP settings after every install — a fresh
    /// install otherwise shows "No MCP servers". See [`ManifestMcpServer`] and
    /// `app::ext::register::register_mcp_servers` on the koma side. Serde-default so a
    /// manifest predating this field parses unchanged, and omitted from the JSON when
    /// empty so an extension with none round-trips byte-identical. See
    /// `docs/EXTENSIONS.md`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<ManifestMcpServer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Free,
    Paid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtensionKind {
    Daemon,
    Oneshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runtime {
    pub exec: String, // path to the executable, relative to the package root
    #[serde(default)]
    pub args: Vec<String>,
}

/// One bundled stdio MCP server declared on [`ExtensionManifest::mcp_servers`] — a
/// standalone MCP binary an extension ships (distinct from `runtime.exec`, which is the
/// extension's own daemon/oneshot process) that koma should register into its MCP
/// catalogue automatically at install time, instead of the user hand-adding it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestMcpServer {
    /// Display name — becomes the registered `McpServerEntry.name` (and thus the
    /// `mcp__<name>__<tool>` advertise prefix), unless it collides with an existing
    /// entry this extension doesn't already own, in which case koma prefixes it with
    /// this extension's id to disambiguate.
    pub name: String,
    /// Path to the stdio MCP server executable, RELATIVE to the package root — the SAME
    /// containment discipline as [`Runtime::exec`] applies: no traversal (`..`, absolute
    /// paths), and it must exist under the extension's install dir after unpack.
    pub exec: String,
    /// Arguments passed to `exec` at spawn.
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Contributes {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub_agents: Vec<SubAgentDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panels: Vec<PanelDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDef>,
    /// koma->ext event names this extension wants delivered via `KomaMsg::Event`
    /// (fire-and-forget; see [`KomaMsg::Event`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    /// OAuth login providers this extension backs (W11). Each becomes a row in
    /// koma's GUI OAuth picker; selecting one delegates the whole login flow to
    /// this extension over the `oauth.*` invoke contract (see [`OAuthProviderDef`]
    /// and the SDK docs). Requires the `oauth:contribute` grant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub oauth_providers: Vec<OAuthProviderDef>,
    /// TUI screens this extension drives via the server-driven TUI SCREEN PROTOCOL
    /// (v1): koma's terminal UI renders each as a full-screen view on the extension's
    /// behalf, exchanging `{ kind: "tui-open" | "tui-select" | "tui-close" }` payloads
    /// over the SAME `panel.msg` invoke + `panel.push` notify verbs a GUI panel uses
    /// (panelId = the screen id), so declaring one is wire-legal with zero protocol
    /// change. Each becomes a selectable row in the `/extension` detail view; opening
    /// it invokes `panel.msg { kind: "tui-open" }` and the extension replies with a
    /// `Screen` to render. See the koma-side `app::ext::screen` module doc for the full
    /// contract. Serde-default so a manifest predating this field parses unchanged, and
    /// omitted from the JSON when empty so an extension with none round-trips
    /// byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tui_screens: Vec<TuiScreenDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentDef {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Tool allow-list this sub-agent should be granted on install. Names must
    /// match koma's selectable tool set (see `agent_selectable_tools()` on the
    /// koma side); unknown names are dropped (not a hard failure) when merged
    /// into the runtime `AgentDef`. Omitted/empty → koma's safe read-only
    /// default (`read`, `grep`, `glob`, `dir_list`), same as before this field
    /// existed. Serde-default so old manifests parse unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDef {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelDef {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub icon: String,
}

/// One TUI screen an extension drives via the server-driven TUI SCREEN PROTOCOL (v1),
/// declared on [`Contributes::tui_screens`]. `id` is the stable screen id koma passes
/// back as the `panelId` on every `panel.msg` invoke (`tui-open` / `tui-select` /
/// `tui-close`) and matches on incoming `panel.push` frames; `title` is the human-facing
/// label shown as the selectable row in koma's `/extension` detail view (and the default
/// header until the extension's first `Screen` supplies its own `title`). Mirrors
/// [`PanelDef`]'s shape (both required).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiScreenDef {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub input_schema: serde_json::Value,
}

/// One OAuth login provider an extension backs (W11 — DELEGATED flow). The
/// extension daemon runs the actual login; koma only contributes the picker row,
/// relays progress phases, and stores the resulting token as an ext-backed
/// connection. See the SDK docs for the `oauth.begin` / `oauth.poll` /
/// `oauth.cancel` invoke contract koma drives this provider through.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthProviderDef {
    /// Stable provider id, unique WITHIN this extension. koma keys the picker row
    /// as `ext:<extension_id>:<id>` and passes `{ "providerId": <id> }` on every
    /// `oauth.*` invoke, so the extension knows which provider a call is for.
    pub id: String,
    /// Human-facing label shown in the picker row.
    pub name: String,
    /// Login shape, mapped to the GUI picker badge kind: `"browser"` → `pkce`
    /// (surface a URL, user opens it), `"device_code"` → `device` (show a user
    /// code + verification URL), `"paste"` → `paste`. Anything else falls back to
    /// the browser badge.
    pub method: String,
    /// W12 (model-provider wiring): the chat-completions endpoint an ext-backed
    /// token resolves to once extension OAuth providers become resolvable model
    /// providers. IGNORED in v1 (account-login/token-storage only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_endpoint: Option<String>,
    /// W12: the wire protocol that endpoint speaks (e.g. `"openai_compatible"`).
    /// IGNORED in v1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_type: Option<String>,
    /// W12: how koma should refresh an expiring ext-backed token itself. IGNORED
    /// in v1 (the extension owns the whole token lifecycle; koma never refreshes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh: Option<OAuthRefreshDef>,
}

/// W12 token-refresh descriptor for an [`OAuthProviderDef`]. IGNORED in v1 — declared
/// now so a v1 manifest that specifies it round-trips the wire without a re-touch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthRefreshDef {
    pub token_url: String,
    pub client_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Grant {
    #[serde(rename = "agents:read")]
    AgentsRead,
    #[serde(rename = "agents:orchestrate")]
    AgentsOrchestrate,
    #[serde(rename = "sessions:manage")]
    SessionsManage,
    #[serde(rename = "chat:prompt")]
    ChatPrompt,
    #[serde(rename = "models:invoke")]
    ModelsInvoke,
    #[serde(rename = "context:publish")]
    ContextPublish,
    /// W11: back one or more OAuth login providers. Gates the host→ext `oauth.*`
    /// delegation invokes (`oauth.begin`/`oauth.poll`/`oauth.cancel`) AND whether
    /// the extension's declared [`Contributes::oauth_providers`] surface as picker
    /// rows. It gates NO ext→koma broker `Call` verb (unlike the other grants),
    /// so it has no `required_grant` entry on koma's side.
    #[serde(rename = "oauth:contribute")]
    OauthContribute,
    /// W12: register/unregister the extension's OWN models into koma's global
    /// catalogue over the `models.register` / `models.unregister` broker verbs.
    /// Unlike [`Self::OauthContribute`] this DOES gate broker `Call` verbs (it has a
    /// `required_grant` entry on koma's side). A model registered under this grant is
    /// served by the extension's connected OAuth account (its `oauth:contribute`
    /// conn), so an extension that registers models almost always requires BOTH.
    #[serde(rename = "models:contribute")]
    ModelsContribute,
    /// Read the user's connected koma.run OAuth token over the general host-broker
    /// `oauth.token` verb (dispatched on a `provider` param — `"koma.run"` today, other
    /// koma.run-backed providers later). This is an ECOSYSTEM primitive: any extension built
    /// on koma.run's service can hold it to read that connection's bearer / expiry / email.
    /// Unlike [`Self::OauthContribute`] (which gates the host→ext delegation invokes and
    /// confers NO broker `Call`), this DOES gate a broker `Call` verb — it has a
    /// `required_grant` entry on koma's side (EXACT-MATCH, like every family grant: it neither
    /// confers nor is conferred by any other).
    #[serde(rename = "oauth:read")]
    OauthRead,
}

// ---- duplex wire envelope ----
// Extension -> koma
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum ExtMsg {
    Hello {
        protocol: String,
        token: String,
        manifest: Box<ExtensionManifest>,
    },
    Call {
        id: u64,
        method: String,
        params: serde_json::Value,
    }, // ext drives koma (requires)
    Result {
        id: u64,
        result: serde_json::Value,
    }, // reply to koma's Invoke
    Health {
        ok: bool,
    },
    // Fire-and-forget ext->koma notification: no `id`, no `Result` reply expected
    // (e.g. `panel.push`). See `Koma::notify` in the SDK.
    Notify {
        name: String,
        params: serde_json::Value,
    },
}

// koma -> extension
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum KomaMsg {
    Welcome {
        protocol: String,
        koma_version: String,
        granted: Vec<Grant>,
    },
    Reject {
        reason: String,
    },
    Invoke {
        id: u64,
        method: String,
        params: serde_json::Value,
    }, // koma drives ext (contributes)
    Result {
        id: u64,
        result: serde_json::Value,
    }, // reply to ext's Call
    Ping,
    Shutdown,
    // Fire-and-forget koma->ext notification: no `id`, no `Result` reply expected.
    // Dispatched to `Extension::on_event`.
    Event {
        name: String,
        params: serde_json::Value,
    },
}

// written by a daemon extension so koma can find its live socket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunInfo {
    pub socket: String,
    pub token: String,
    pub pid: u32,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    /// (a) An old-style manifest fragment with a bare `SubAgentDef {name, description}`
    /// and no `events` key must still parse — the new fields are all additive/optional.
    #[test]
    fn old_style_contributes_without_events_or_subagent_extras_parses() {
        let raw = r#"{
            "sub_agents": [ { "name": "planner", "description": "plans things" } ]
        }"#;
        let c: Contributes = serde_json::from_str(raw).expect("old-style contributes parses");
        assert_eq!(c.sub_agents.len(), 1);
        assert_eq!(c.sub_agents[0].name, "planner");
        assert_eq!(c.sub_agents[0].description, "plans things");
        assert!(c.sub_agents[0].prompt.is_none());
        assert!(c.sub_agents[0].model.is_none());
        assert!(c.sub_agents[0].effort.is_none());
        assert!(c.events.is_empty());
        // W11: a manifest predating `oauth_providers` still parses (additive/optional).
        assert!(c.oauth_providers.is_empty());
        // A manifest predating `tui_screens` still parses (additive/optional).
        assert!(c.tui_screens.is_empty());
    }

    /// (W11) An [`OAuthProviderDef`] round-trips through serde with the W12 fields
    /// omitted when absent (so a v1 manifest stays minimal), and preserved when present.
    #[test]
    fn oauth_provider_def_roundtrips() {
        // Minimal v1 form: id/name/method only. The W12 option fields must be OMITTED.
        let raw = r#"{ "id": "github", "name": "GitHub", "method": "device_code" }"#;
        let def: OAuthProviderDef = serde_json::from_str(raw).expect("minimal def parses");
        assert_eq!(def.id, "github");
        assert_eq!(def.name, "GitHub");
        assert_eq!(def.method, "device_code");
        assert!(def.chat_endpoint.is_none());
        assert!(def.api_type.is_none());
        assert!(def.refresh.is_none());
        let wire = serde_json::to_value(&def).expect("serializes");
        assert_eq!(
            wire.get("chat_endpoint"),
            None,
            "absent W12 fields must not serialize"
        );
        assert_eq!(wire.get("refresh"), None);

        // Full form: the W12 fields (ignored in v1) still round-trip.
        let def2 = OAuthProviderDef {
            id: "acme".to_string(),
            name: "Acme".to_string(),
            method: "browser".to_string(),
            chat_endpoint: Some("https://api.acme.test/v1".to_string()),
            api_type: Some("openai_compatible".to_string()),
            refresh: Some(OAuthRefreshDef {
                token_url: "https://acme.test/token".to_string(),
                client_id: "cid".to_string(),
            }),
        };
        let back: OAuthProviderDef = serde_json::from_value(serde_json::to_value(&def2).unwrap())
            .expect("full def roundtrips");
        assert_eq!(
            back.chat_endpoint.as_deref(),
            Some("https://api.acme.test/v1")
        );
        assert_eq!(
            back.refresh.as_ref().unwrap().token_url,
            "https://acme.test/token"
        );
    }

    /// (b) `KomaMsg::Event` roundtrips through serde_json and tags as "event".
    #[test]
    fn koma_msg_event_roundtrips() {
        let msg = KomaMsg::Event {
            name: "focus.changed".to_string(),
            params: json!({ "id": 42 }),
        };
        let wire = serde_json::to_value(&msg).expect("serializes");
        assert_eq!(wire["t"], "event");
        assert_eq!(wire["name"], "focus.changed");
        assert_eq!(wire["params"]["id"], 42);

        let back: KomaMsg = serde_json::from_value(wire).expect("deserializes");
        match back {
            KomaMsg::Event { name, params } => {
                assert_eq!(name, "focus.changed");
                assert_eq!(params, json!({ "id": 42 }));
            }
            other => panic!("expected KomaMsg::Event, got {other:?}"),
        }
    }

    /// (b) `ExtMsg::Notify` roundtrips through serde_json and tags as "notify".
    #[test]
    fn ext_msg_notify_roundtrips() {
        let msg = ExtMsg::Notify {
            name: "panel.push".to_string(),
            params: json!({ "panelId": "sidebar", "payload": { "ok": true } }),
        };
        let wire = serde_json::to_value(&msg).expect("serializes");
        assert_eq!(wire["t"], "notify");
        assert_eq!(wire["name"], "panel.push");

        let back: ExtMsg = serde_json::from_value(wire).expect("deserializes");
        match back {
            ExtMsg::Notify { name, params } => {
                assert_eq!(name, "panel.push");
                assert_eq!(
                    params,
                    json!({ "panelId": "sidebar", "payload": { "ok": true } })
                );
            }
            other => panic!("expected ExtMsg::Notify, got {other:?}"),
        }
    }

    /// (c) A frame with an unknown tag fails to parse cleanly (Err, not a panic).
    #[test]
    fn unknown_tag_fails_cleanly() {
        let raw = r#"{"t":"bogus","name":"x"}"#;
        let koma_result: Result<KomaMsg, _> = serde_json::from_str(raw);
        assert!(koma_result.is_err());

        let ext_result: Result<ExtMsg, _> = serde_json::from_str(raw);
        assert!(ext_result.is_err());
    }

    /// (d) The four new `Grant` variants roundtrip to their documented wire strings.
    #[test]
    fn grant_new_variants_roundtrip_wire_strings() {
        let cases = [
            (Grant::SessionsManage, "\"sessions:manage\""),
            (Grant::ChatPrompt, "\"chat:prompt\""),
            (Grant::ModelsInvoke, "\"models:invoke\""),
            (Grant::ContextPublish, "\"context:publish\""),
            (Grant::OauthContribute, "\"oauth:contribute\""),
            (Grant::ModelsContribute, "\"models:contribute\""),
        ];
        for (grant, wire) in cases {
            let serialized = serde_json::to_string(&grant).expect("serializes");
            assert_eq!(serialized, wire);
            let back: Grant = serde_json::from_str(wire).expect("deserializes");
            assert_eq!(back, grant);
        }
    }
}

// W13: additional regression suite — pure addition, sibling file, never touches the `tests`
// module above.
#[cfg(test)]
#[path = "protocol_test.rs"]
mod protocol_test;
