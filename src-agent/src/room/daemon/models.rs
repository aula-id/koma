//! Which model each lane runs.
//!
//! The choices are the models registered in the coding agent (the session's own
//! list first, then the global catalogue), so the room never invents a model the
//! user has not set up. A choice is stored in the session overlay's
//! `avatars.toml`, where the registry already reads per-lane models, and the
//! registry is reloaded so the lane's next turn uses it. A running turn is not
//! changed.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::model::app_config::AppConfig;
use crate::model::settings::Settings;
use crate::room::registry::{blocks_avatar_write, LaneKind, RegistryHandle};
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

    /// One row per lane that exists: the chief has a chat lane only and the
    /// clerk a work lane only.
    pub fn lanes(&self) -> Vec<LaneRow> {
        let registry = self.registry.current();
        let mut rows = Vec::new();
        for avatar in registry.avatars.keys() {
            for (kind, name) in [(LaneKind::Chat, "chat"), (LaneKind::Work, "work")] {
                if !lane_exists(avatar, kind) {
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

    /// Re-read the registry files so the next lane turn uses them, and return
    /// the registry version and the warnings of that load. A running turn keeps
    /// its snapshot.
    pub fn reload_registry(&self) -> (u64, Vec<String>) {
        self.registry.reload();
        (self.registry.current().version, self.registry.report().warnings)
    }

    /// The revision of the template layer the room read at start.
    pub fn template_revision(&self) -> Option<String> {
        self.registry.template_revision()
    }

    /// Point one lane at a registered model, or back at the session default when
    /// `choice` is empty. The new file is checked before it is written, and the
    /// old one is put back when the reloaded room does not show the change.
    pub fn set(&self, avatar: &str, lane: &str, choice: &str) -> Result<(), String> {
        self.set_then(avatar, lane, choice, |_| {})
    }

    /// [`ModelBook::set`], calling `after_write` with the file's path between the
    /// write and the reload.
    fn set_then(&self, avatar: &str, lane: &str, choice: &str, after_write: impl FnOnce(&Path)) -> Result<(), String> {
        let registry = self.registry.current();
        let Some(member) = registry.avatar(avatar) else {
            return Err(format!("no avatar named '{avatar}'"));
        };
        let kind = match lane {
            "chat" => LaneKind::Chat,
            "work" => LaneKind::Work,
            _ => return Err(format!("a lane is chat or work, not '{lane}'")),
        };
        if !lane_exists(avatar, kind) {
            return Err(match kind {
                LaneKind::Chat => format!("the {avatar} has only a work lane"),
                LaneKind::Work => format!("the {avatar} has only a chat lane"),
            });
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
        let before = match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("reading {}: {e}", path.display())),
        };
        let mut doc: toml::Table = match &before {
            Some(bytes) => {
                let text = std::str::from_utf8(bytes).map_err(|e| format!("avatars.toml: {e}"))?;
                toml::from_str(text).map_err(|e| format!("avatars.toml: {e}"))?
            }
            None => toml::Table::new(),
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
        let wanted = match picked {
            None => {
                entry.remove(lane);
                None
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
                let table = toml::Value::Table(table);
                entry.insert(lane.to_string(), table.clone());
                Some(table)
            }
        };

        let text = toml::to_string(&doc).map_err(|e| e.to_string())?;
        if let Some(warning) = self
            .registry
            .check_overlay_file("avatars.toml", &text)
            .into_iter()
            .find(|w| blocks_avatar_write(w, avatar))
        {
            return Err(format!("avatars.toml is not valid: {warning}; nothing was changed"));
        }

        std::fs::create_dir_all(&self.overlay).map_err(|e| format!("creating {}: {e}", self.overlay.display()))?;
        self.write_file(&path, text.as_bytes())?;
        after_write(&path);
        self.registry.reload();

        // The file must hold the intended entry, and the reloaded room must have
        // read the file and applied that entry.
        let accepted = written_lane(&path, avatar, lane).is_some_and(|lane| lane == wanted)
            && !self
                .registry
                .report()
                .warnings
                .iter()
                .any(|w| blocks_avatar_write(w, avatar) || self.registry.drops_overlay_file(w, "avatars.toml"));
        if !accepted {
            let restored = match &before {
                Some(bytes) => self.write_file(&path, bytes),
                None => match std::fs::remove_file(&path) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        Err(format!("removing {}: {e}", path.display()))
                    }
                    _ => Ok(()),
                },
            };
            self.registry.reload();
            return Err(match restored {
                Ok(()) => "the room did not accept that change; nothing was changed".into(),
                Err(e) => format!("the room did not accept that change, and avatars.toml could not be restored: {e}"),
            });
        }
        Ok(())
    }

    fn write_file(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        // Write beside the file and rename over it, so a reader never sees half of it. Windows can
        // refuse the rename (a watcher holds the new file, or the call needs a privilege the runner
        // lacks), so there the file is written in place: it is a few lines, and the choice is kept.
        let tmp = self.overlay.join("avatars.toml.tmp");
        let swapped = !cfg!(windows)
            && std::fs::write(&tmp, bytes).is_ok()
            && (0..5).any(|attempt| {
                let ok = std::fs::rename(&tmp, path).is_ok();
                if !ok {
                    std::thread::sleep(std::time::Duration::from_millis(40 * (attempt + 1)));
                }
                ok
            });
        if !swapped {
            let _ = std::fs::remove_file(&tmp);
            std::fs::write(path, bytes).map_err(|e| format!("writing {}: {e}", path.display()))?;
        }
        Ok(())
    }
}

/// Whether `avatar` has this lane at all. The chief only talks, so it has no
/// work lane; the clerk only works, so it has no chat lane.
fn lane_exists(avatar: &str, lane: LaneKind) -> bool {
    !matches!((avatar, lane), ("chief", LaneKind::Work) | ("clerk", LaneKind::Chat))
}

/// The lane table `avatar`'s entry in the file at `path` holds now: `Some(None)`
/// when the entry has no table for `lane`, `None` when the file cannot be read
/// or has no entry for `avatar`.
fn written_lane(path: &Path, avatar: &str, lane: &str) -> Option<Option<toml::Value>> {
    let text = std::fs::read_to_string(path).ok()?;
    let doc: toml::Table = toml::from_str(&text).ok()?;
    let entry = doc
        .get("avatar")?
        .as_array()?
        .iter()
        .find(|v| v.get("id").and_then(|id| id.as_str()) == Some(avatar))?;
    Some(entry.get(lane).cloned())
}

#[cfg(test)]
pub(crate) fn test_book(dir: &std::path::Path) -> Arc<ModelBook> {
    let mut config = AppConfig {
        providers: vec![serde_json::from_value(serde_json::json!({"uuid": "p1", "name": "acme"})).expect("provider")],
        ..AppConfig::default()
    };
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
        assert!(rows.iter().any(|r| r.avatar == "chief" && r.lane == "chat"));
        assert!(!rows.iter().any(|r| r.avatar == "chief" && r.lane == "work"));
        assert!(rows.iter().all(|r| r.model == "session default"));
    }

    #[test]
    fn lanes_that_do_not_exist_are_refused_before_any_file_is_touched() {
        let tmp = dir("no-lane");
        let book = test_book(&tmp);
        assert_eq!(book.set("chief", "work", "m1"), Err("the chief has only a chat lane".to_string()));
        assert_eq!(book.set("clerk", "chat", "m1"), Err("the clerk has only a work lane".to_string()));
        assert_eq!(book.set("chief", "work", ""), Err("the chief has only a chat lane".to_string()));
        assert!(!tmp.join("room").exists(), "no overlay was created");
        book.set("chief", "chat", "m1").expect("the chief's chat lane");
        book.set("clerk", "work", "m2").expect("the clerk's work lane");
    }

    #[test]
    fn a_file_the_loader_would_drop_is_not_written() {
        let tmp = dir("dup");
        let overlay = tmp.join("room");
        std::fs::create_dir_all(&overlay).expect("mkdir");
        let body = "[[avatar]]\nid = \"red-2\"\nrole = \"red\"\n\n[[avatar]]\nid = \"red-2\"\nrole = \"red\"\n";
        std::fs::write(overlay.join("avatars.toml"), body).expect("write");
        let book = test_book(&tmp);
        let err = book.set("red", "work", "m1").expect_err("refused");
        assert!(err.starts_with("avatars.toml is not valid: ") && err.ends_with("; nothing was changed"), "{err}");
        assert!(err.contains("defined twice"), "{err}");
        assert_eq!(std::fs::read_to_string(overlay.join("avatars.toml")).expect("read"), body);
        assert_eq!(model_of(&book, "red", "work"), "session default");
    }

    #[test]
    fn a_change_the_room_does_not_show_is_rolled_back() {
        // A hand edit lands between the write and the reload.
        let tmp = dir("rollback");
        let overlay = tmp.join("room");
        std::fs::create_dir_all(&overlay).expect("mkdir");
        let body = "[[avatar]]\nid = \"red-api\"\nrole = \"red\"\n";
        std::fs::write(overlay.join("avatars.toml"), body).expect("write");
        let book = test_book(&tmp);
        let clobber = |path: &Path| std::fs::write(path, "[[avatar]]\nid = \"red\"\nrole = \"red\"\n").expect("edit");
        assert_eq!(
            book.set_then("red", "work", "m1", clobber),
            Err("the room did not accept that change; nothing was changed".to_string())
        );
        assert_eq!(std::fs::read_to_string(overlay.join("avatars.toml")).expect("read"), body);
        assert!(book.registry.current().avatar("red-api").is_some());
        assert_eq!(model_of(&book, "red", "work"), "session default");

        // With no file before, the rollback removes it again.
        let tmp = dir("rollback-absent");
        let book = test_book(&tmp);
        assert!(book.set_then("red", "work", "m1", |path| std::fs::write(path, "").expect("edit")).is_err());
        assert!(!tmp.join("room").join("avatars.toml").exists());
        assert_eq!(model_of(&book, "red", "work"), "session default");
    }

    #[test]
    fn a_stale_entry_does_not_block_another_avatars_lane() {
        let tmp = dir("stale");
        let overlay = tmp.join("room");
        std::fs::create_dir_all(&overlay).expect("mkdir");
        std::fs::write(overlay.join("avatars.toml"), "[[avatar]]\nid = \"ghost\"\nrole = \"phantom\"\n").expect("write");
        let book = test_book(&tmp);
        assert_eq!(book.registry.report().warnings.len(), 1);
        book.set("red", "work", "m2").expect("set");
        assert_eq!(model_of(&book, "red", "work"), "Strong");
        let text = std::fs::read_to_string(overlay.join("avatars.toml")).expect("read");
        assert!(text.contains("phantom"), "{text}");
        // The stale entry itself still cannot be set.
        assert!(book.set("ghost", "work", "m1").is_err());
    }

    #[test]
    fn an_entry_the_loader_already_ignores_cannot_be_set() {
        for (body, warning) in [
            (
                "[[avatar]]\nid = \"red-web\"\nrole = \"phantom\"\n",
                "avatars.toml: avatar 'red-web' uses unknown role 'phantom'; ignored",
            ),
            (
                "[[avatar]]\nid = \"red-web\"\nrole = \"blue\"\n",
                "avatars.toml: avatar 'red-web' cannot change its role; ignored",
            ),
        ] {
            let tmp = dir("ignored-entry");
            let overlay = tmp.join("room");
            std::fs::create_dir_all(&overlay).expect("mkdir");
            std::fs::write(overlay.join("avatars.toml"), body).expect("write");
            let book = test_book(&tmp);
            assert!(book.registry.current().avatar("red-web").is_some());
            assert_eq!(
                book.set("red-web", "work", "m1"),
                Err(format!("avatars.toml is not valid: {warning}; nothing was changed"))
            );
            assert_eq!(std::fs::read_to_string(overlay.join("avatars.toml")).expect("read"), body);
            assert_eq!(model_of(&book, "red-web", "work"), "session default");
            // The other avatars' lanes are still settable.
            book.set("red", "work", "m1").expect("set");
        }
    }

    #[test]
    fn a_file_the_room_does_not_read_is_put_back() {
        // A file over the overlay size limit is skipped when the room reads it.
        let tmp = dir("too-large");
        let overlay = tmp.join("room");
        std::fs::create_dir_all(&overlay).expect("mkdir");
        let body = format!("[[avatar]]\nid = \"red-big\"\nrole = \"red\"\ndisplay = \"{}\"\n", "x".repeat(300 * 1024));
        std::fs::write(overlay.join("avatars.toml"), &body).expect("write");
        let book = test_book(&tmp);
        assert_eq!(
            book.set("red", "work", "m1"),
            Err("the room did not accept that change; nothing was changed".to_string())
        );
        assert_eq!(std::fs::read_to_string(overlay.join("avatars.toml")).expect("read"), body);
        assert_eq!(model_of(&book, "red", "work"), "session default");
    }

    #[test]
    fn a_failed_rollback_is_reported() {
        // The new file is replaced by a directory, which cannot be removed as a file.
        let tmp = dir("rollback-fails");
        let book = test_book(&tmp);
        let block = |path: &Path| {
            std::fs::remove_file(path).expect("remove");
            std::fs::create_dir(path).expect("mkdir");
        };
        let err = book.set_then("red", "work", "m1", block).expect_err("refused");
        assert!(
            err.starts_with("the room did not accept that change, and avatars.toml could not be restored: "),
            "{err}"
        );
        assert!(!err.contains("nothing was changed"), "{err}");
    }

    #[test]
    fn a_reset_and_a_bare_model_succeed_over_role_lane_defaults() {
        let tmp = dir("role-defaults");
        let overlay = tmp.join("room");
        let embedded = RegistryHandle::embedded().expect("load").current();
        let red = embedded.role("red").expect("red");
        let role = format!(
            "id = \"red\"\ndisplay = \"{}\"\nshelf_allow = {:?}\n[chat]\nmodel = \"role-chat\"\n[work]\nprovider = \"p9\"\nmodel = \"role-work\"\n",
            red.display, red.shelf_allow
        );
        std::fs::create_dir_all(overlay.join("roles").join("red")).expect("mkdir");
        std::fs::write(overlay.join("roles").join("red").join("role.toml"), role).expect("write");

        let mut config = AppConfig {
            providers: vec![serde_json::from_value(serde_json::json!({"uuid": "p1", "name": "acme"})).expect("provider")],
            ..AppConfig::default()
        };
        config.models = vec![
            serde_json::from_value(serde_json::json!({"uuid": "m1", "name": "Fast", "model_id": "fast-1", "provider_uuid": "p1"})).expect("model"),
            serde_json::from_value(serde_json::json!({"uuid": "m3", "name": "Local", "model_id": "local-1", "provider_uuid": ""})).expect("model"),
        ];
        let registry = Arc::new(RegistryHandle::with_overlay(overlay.clone()).expect("registry"));
        assert!(registry.report().warnings.is_empty(), "{:?}", registry.report().warnings);
        let book = ModelBook::new(&config, &Settings::default(), registry, overlay.clone());

        book.set("red", "work", "m3").expect("bare model");
        let text = std::fs::read_to_string(overlay.join("avatars.toml")).expect("read");
        assert!(text.contains("model = \"local-1\"") && !text.contains("provider"), "{text}");
        book.set("red", "work", "").expect("reset");
        assert_eq!(model_of(&book, "red", "work"), "role-work");
        book.set("red", "chat", "m1").expect("set");
        book.set("red", "chat", "").expect("reset");
        assert_eq!(model_of(&book, "red", "chat"), "role-chat");
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
