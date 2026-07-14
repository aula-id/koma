use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: &str = "v0";
pub const MANIFEST_SCHEMA: &str = "koma-extension/v0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub schema: String,           // MANIFEST_SCHEMA
    pub id: String,               // reverse-DNS, e.g. "run.koma.example.echo-tool-daemon"
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier { Free, Paid }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtensionKind { Daemon, Oneshot }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runtime {
    pub exec: String,             // path to the executable, relative to the package root
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDef { pub id: String, pub display_name: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelDef { pub id: String, pub title: String, #[serde(default)] pub icon: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub input_schema: serde_json::Value,
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
}

// ---- duplex wire envelope ----
// Extension -> koma
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum ExtMsg {
    Hello { protocol: String, token: String, manifest: ExtensionManifest },
    Call { id: u64, method: String, params: serde_json::Value },   // ext drives koma (requires)
    Result { id: u64, result: serde_json::Value },                 // reply to koma's Invoke
    Health { ok: bool },
    // Fire-and-forget ext->koma notification: no `id`, no `Result` reply expected
    // (e.g. `panel.push`). See `Koma::notify` in the SDK.
    Notify { name: String, params: serde_json::Value },
}

// koma -> extension
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum KomaMsg {
    Welcome { protocol: String, koma_version: String, granted: Vec<Grant> },
    Reject { reason: String },
    Invoke { id: u64, method: String, params: serde_json::Value },  // koma drives ext (contributes)
    Result { id: u64, result: serde_json::Value },                  // reply to ext's Call
    Ping,
    Shutdown,
    // Fire-and-forget koma->ext notification: no `id`, no `Result` reply expected.
    // Dispatched to `Extension::on_event`.
    Event { name: String, params: serde_json::Value },
}

// written by a daemon extension so koma can find its live socket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunInfo { pub socket: String, pub token: String, pub pid: u32 }

#[cfg(test)]
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
    }

    /// (b) `KomaMsg::Event` roundtrips through serde_json and tags as "event".
    #[test]
    fn koma_msg_event_roundtrips() {
        let msg = KomaMsg::Event { name: "focus.changed".to_string(), params: json!({ "id": 42 }) };
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
                assert_eq!(params, json!({ "panelId": "sidebar", "payload": { "ok": true } }));
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
        ];
        for (grant, wire) in cases {
            let serialized = serde_json::to_string(&grant).expect("serializes");
            assert_eq!(serialized, wire);
            let back: Grant = serde_json::from_str(wire).expect("deserializes");
            assert_eq!(back, grant);
        }
    }
}
