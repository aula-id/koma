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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentDef { pub name: String, pub description: String }

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
}

// written by a daemon extension so koma can find its live socket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunInfo { pub socket: String, pub token: String, pub pid: u32 }
