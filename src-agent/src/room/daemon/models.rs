//! Which model each lane runs.
//!
//! The choices are the models registered in the coding agent (the session's own
//! list first, then the global catalogue), so the room never invents a model the
//! user has not set up. A choice is stored in the session overlay's
//! `avatars.toml`, where the registry already reads per-lane models, and the
//! registry is reloaded so the lane's next turn uses it. A running turn is not
//! changed.

use std::path::PathBuf;
use std::sync::Arc;

use crate::model::app_config::AppConfig;
use crate::model::settings::Settings;
use crate::room::registry::{LaneKind, RegistryHandle};
use crate::room::wire::{LaneRow, ModelChoice};

pub struct ModelBook {
    choices: Vec<(ModelChoice, String)>,
    registry: Arc<RegistryHandle>,
    overlay: PathBuf,
}

impl ModelBook {
    pub fn new(config: &AppConfig, settings: &Settings, registry: Arc<RegistryHandle>, overlay: PathBuf) -> Self {
        let mut choices: Vec<(ModelChoice, String)> = Vec::new();
        for entry in settings.session_models.iter().chain(config.models.iter()) {
            // A session copy of a global entry stands for it once.
            let id = entry.source_uuid.clone().unwrap_or_else(|| entry.uuid.clone());
            if choices.iter().any(|(c, _)| c.id == id || c.id == entry.uuid) {
                continue;
            }
            let provider = config
                .providers
                .iter()
                .find(|p| p.uuid == entry.provider_uuid)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            let name = if entry.name.trim().is_empty() { entry.model_id.clone() } else { entry.name.clone() };
            choices.push((
                ModelChoice { id, name, model_id: entry.model_id.clone(), provider },
                entry.provider_uuid.clone(),
            ));
        }
        Self { choices, registry, overlay }
    }

    pub fn choices(&self) -> Vec<ModelChoice> {
        self.choices.iter().map(|(c, _)| c.clone()).collect()
    }

    /// One row per lane: the clerk has a work lane only.
    pub fn lanes(&self) -> Vec<LaneRow> {
        let registry = self.registry.current();
        let mut rows = Vec::new();
        for avatar in registry.avatars.keys() {
            for (kind, name) in [(LaneKind::Chat, "chat"), (LaneKind::Work, "work")] {
                if kind == LaneKind::Chat && avatar == "clerk" {
                    continue;
                }
                let lane = registry.lane_model(avatar, kind);
                let label = match lane.model.as_deref().filter(|m| !m.is_empty()) {
                    None => "session default".to_string(),
                    Some(model) => {
                        let shown = self
                            .choices
                            .iter()
                            .find(|(c, _)| c.id == model || c.model_id == model || c.name == model)
                            .map(|(c, _)| c.name.clone())
                            .unwrap_or_else(|| model.to_string());
                        match lane.effort.as_deref().filter(|e| !e.is_empty()) {
                            Some(effort) => format!("{shown} · {effort}"),
                            None => shown,
                        }
                    }
                };
                rows.push(LaneRow {
                    avatar: avatar.clone(),
                    display: registry.display_of(avatar).to_string(),
                    lane: name.to_string(),
                    model: label,
                });
            }
        }
        rows
    }

    /// Point one lane at a registered model, or back at the session default when
    /// `choice` is empty.
    pub fn set(&self, avatar: &str, lane: &str, choice: &str) -> Result<(), String> {
        let registry = self.registry.current();
        let Some(member) = registry.avatar(avatar) else {
            return Err(format!("no avatar named '{avatar}'"));
        };
        if lane != "chat" && lane != "work" {
            return Err(format!("a lane is chat or work, not '{lane}'"));
        }
        let picked = if choice.is_empty() {
            None
        } else {
            Some(
                self.choices
                    .iter()
                    .find(|(c, _)| c.id == choice)
                    .ok_or_else(|| "that model is not registered".to_string())?,
            )
        };

        let path = self.overlay.join("avatars.toml");
        let mut doc: toml::Table = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("avatars.toml: {e}"))?,
            Err(_) => toml::Table::new(),
        };
        let list = doc.entry("avatar").or_insert_with(|| toml::Value::Array(Vec::new()));
        let toml::Value::Array(list) = list else {
            return Err("avatars.toml: `avatar` is not a list".into());
        };
        let at = list.iter().position(|v| v.get("id").and_then(|id| id.as_str()) == Some(avatar));
        let at = at.unwrap_or_else(|| {
            let mut fresh = toml::Table::new();
            fresh.insert("id".into(), avatar.to_string().into());
            fresh.insert("role".into(), member.role.clone().into());
            list.push(toml::Value::Table(fresh));
            list.len() - 1
        });
        let toml::Value::Table(entry) = &mut list[at] else {
            return Err("avatars.toml: bad avatar entry".into());
        };
        match picked {
            None => {
                entry.remove(lane);
            }
            Some((model, provider_uuid)) => {
                let mut table = toml::Table::new();
                if provider_uuid.is_empty() {
                    table.insert("model".into(), model.model_id.clone().into());
                } else {
                    table.insert("provider".into(), provider_uuid.clone().into());
                    table.insert("model".into(), model.id.clone().into());
                }
                // Keep the effort a lane already had.
                if let Some(effort) = entry.get(lane).and_then(|v| v.get("effort")).cloned() {
                    table.insert("effort".into(), effort);
                }
                entry.insert(lane.to_string(), toml::Value::Table(table));
            }
        }

        std::fs::create_dir_all(&self.overlay).map_err(|e| e.to_string())?;
        let text = toml::to_string(&doc).map_err(|e| e.to_string())?;
        let tmp = self.overlay.join("avatars.toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        self.registry.reload();
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn test_book(dir: &std::path::Path) -> Arc<ModelBook> {
    let mut config = AppConfig::default();
    config.providers = vec![serde_json::from_value(serde_json::json!({"uuid": "p1", "name": "acme"})).expect("provider")];
    config.models = vec![
        serde_json::from_value(serde_json::json!({"uuid": "m1", "name": "Fast", "model_id": "fast-1", "provider_uuid": "p1"})).expect("model"),
        serde_json::from_value(serde_json::json!({"uuid": "m2", "name": "Strong", "model_id": "strong-1", "provider_uuid": "p1"})).expect("model"),
    ];
    let overlay = dir.join("room");
    let registry = Arc::new(RegistryHandle::with_overlay(overlay.clone()).expect("registry"));
    Arc::new(ModelBook::new(&config, &Settings::default(), registry, overlay))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("koma-room-models-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn model_of(book: &ModelBook, avatar: &str, lane: &str) -> String {
        book.lanes().into_iter().find(|r| r.avatar == avatar && r.lane == lane).expect("row").model
    }

    #[test]
    fn registered_models_are_the_only_choices() {
        let book = test_book(&dir("choices"));
        let names: Vec<String> = book.choices().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["Fast", "Strong"]);
        assert_eq!(book.choices()[0].provider, "acme");
    }

    #[test]
    fn every_avatar_lists_its_lanes_and_the_clerk_has_only_work() {
        let rows = test_book(&dir("rows")).lanes();
        assert!(rows.iter().any(|r| r.avatar == "red" && r.lane == "chat"));
        assert!(rows.iter().any(|r| r.avatar == "red" && r.lane == "work"));
        assert!(rows.iter().any(|r| r.avatar == "clerk" && r.lane == "work"));
        assert!(!rows.iter().any(|r| r.avatar == "clerk" && r.lane == "chat"));
        assert!(rows.iter().all(|r| r.model == "session default"));
    }

    #[test]
    fn a_choice_sticks_to_its_lane_only_and_can_be_reset() {
        let book = test_book(&dir("set"));
        book.set("red", "work", "m2").expect("set");
        assert_eq!(model_of(&book, "red", "work"), "Strong");
        assert_eq!(model_of(&book, "red", "chat"), "session default");
        book.set("red", "chat", "m1").expect("set chat");
        assert_eq!(model_of(&book, "red", "work"), "Strong");
        assert_eq!(model_of(&book, "red", "chat"), "Fast");
        book.set("red", "work", "").expect("reset");
        assert_eq!(model_of(&book, "red", "work"), "session default");
        assert_eq!(model_of(&book, "red", "chat"), "Fast");
        // The lane resolves to the provider connection and the registered model.
        let lane = book.registry.current().lane_model("red", LaneKind::Chat);
        assert_eq!(lane.provider.as_deref(), Some("p1"));
        assert_eq!(lane.model.as_deref(), Some("m1"));
    }

    #[test]
    fn unknown_models_avatars_and_lanes_are_refused_and_change_nothing() {
        let book = test_book(&dir("refuse"));
        assert!(book.set("red", "work", "nope").is_err());
        assert!(book.set("ghost", "work", "m1").is_err());
        assert!(book.set("red", "sleep", "m1").is_err());
        assert!(book.lanes().iter().all(|r| r.model == "session default"));
    }

    #[test]
    fn other_overlay_entries_survive_a_choice() {
        let tmp = dir("keep");
        let overlay = tmp.join("room");
        std::fs::create_dir_all(&overlay).expect("mkdir");
        std::fs::write(overlay.join("avatars.toml"), "[[avatar]]\nid = \"red-api\"\nrole = \"red\"\n").expect("write");
        let book = test_book(&tmp);
        book.set("reviewer", "work", "m1").expect("set");
        let text = std::fs::read_to_string(overlay.join("avatars.toml")).expect("read");
        assert!(text.contains("red-api"), "{text}");
        assert!(book.registry.current().avatar("red-api").is_some());
    }
}
