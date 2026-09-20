//! Credential-free, daemon-authoritative state for `koma run` setup and inspection.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunState {
    pub session_id: String,
    pub name: String,
    pub workdir: Vec<String>,
    pub model: String,
    pub effort: String,
    pub mode: String,
    pub working: bool,
    pub awaiting_approval: bool,
    pub security_enabled: bool,
    pub security_running: bool,
    pub yolo_armed: bool,
    pub short_send: bool,
    pub drss_active: bool,
    pub max_output_tokens: u32,
    pub context_window_limit: u64,
    pub context_model_alias: String,
    pub extensions: Vec<RunExtension>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunExtension {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub activation: String,
    pub enabled: bool,
    pub active: bool,
    pub running: bool,
}
