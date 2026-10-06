use crate::app::state::AppState;
use crate::ipc::proto::{ClientRequest, DaemonEvent};
use crate::model::skill::{SkillItemOutcome, SkillRegistry};

use super::core::{DaemonHub, StoreReply};

fn registry_for(state: &AppState) -> SkillRegistry {
    let workdir = state
        .rest
        .fg()
        .session
        .as_ref()
        .map(crate::model::session::Session::workdir);
    SkillRegistry::load(workdir.as_deref(), &state.rest.config.extra_skill_roots)
}

fn loaded_names(state: &AppState) -> Vec<String> {
    let foreground = state.rest.fg();
    if foreground.active_skills.is_empty() {
        foreground
            .projected_loaded_skill_names
            .iter()
            .cloned()
            .collect()
    } else {
        foreground.active_skills.keys().cloned().collect()
    }
}

impl DaemonHub {
    pub(super) fn list_skills(
        &mut self,
        idx: usize,
        state: &AppState,
        request_id: String,
        session_epoch: u64,
    ) {
        self.send_to(
            idx,
            DaemonEvent::SkillValues {
                request_id,
                session_epoch,
                skills: registry_for(state).catalogue(),
                loaded_skill_names: loaded_names(state),
                error: None,
            },
        );
    }

    pub(super) fn set_extra_skill_roots(
        &mut self,
        idx: usize,
        state: &mut AppState,
        roots: Vec<String>,
        request_id: String,
        session_epoch: u64,
        tab_id: String,
    ) {
        let requested: Vec<std::path::PathBuf> = roots
            .into_iter()
            .map(|root| root.trim().to_string())
            .filter(|root| !root.is_empty())
            .map(std::path::PathBuf::from)
            .collect();
        let workdir = state
            .rest
            .fg()
            .session
            .as_ref()
            .map(crate::model::session::Session::workdir);
        let valid = crate::model::skill::valid_extra_skill_roots(workdir.as_deref(), &requested);
        let safe_for_every_project = state.rest.sessions.iter().all(|runtime| {
            runtime.session.as_ref().is_none_or(|session| {
                crate::model::skill::valid_extra_skill_roots(Some(&session.workdir()), &requested)
                    .len()
                    == requested.len()
            })
        });
        let result = if valid.len() != requested.len() || !safe_for_every_project {
            Err(anyhow::anyhow!(
                "Every External skill root must exist, be unique, and not overlap a Koma-owned Global or Project skills directory"
            ))
        } else {
            let previous = state.rest.config.extra_skill_roots.clone();
            state.rest.config.extra_skill_roots = valid.into_iter().map(|root| root.path).collect();
            let saved = crate::app::runtime::actions::save_config_and_broadcast(&state.rest.config);
            if saved.is_err() {
                state.rest.config.extra_skill_roots = previous;
            }
            saved
        };
        if result.is_ok() {
            let config = state.rest.config.clone();
            for runtime in &mut state.rest.sessions {
                let Some(session) = runtime.session.as_mut() else {
                    continue;
                };
                session.rebuild_system_with(
                    &crate::model::agent_def::load_registry(Some(&session.path)),
                    &config,
                );
            }
            self.force_resync = true;
        }
        self.send_to(
            idx,
            DaemonEvent::SkillOp {
                request_id: request_id.clone(),
                session_epoch,
                tab_id,
                operation: "set-roots".to_string(),
                outcomes: vec![SkillItemOutcome {
                    name: "External skill roots".to_string(),
                    status: if result.is_ok() { "success" } else { "failed" }.to_string(),
                    error: result.err().map(|error| error.to_string()),
                }],
                loaded_skill_names: loaded_names(state),
            },
        );
        self.list_skills(idx, state, request_id, session_epoch);
        self.send_settings_values(idx, state);
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn get_skill_detail(
        &mut self,
        idx: usize,
        state: &AppState,
        skill_id: String,
        generation: String,
        request_id: String,
        session_epoch: u64,
        tab_id: String,
    ) {
        let registry = registry_for(state);
        let (detail, error) = match registry.detail_by_identity(&skill_id, &generation) {
            Ok(detail) => (Some(detail), None),
            Err(error) => (None, Some(error.to_string())),
        };
        self.send_to(
            idx,
            DaemonEvent::SkillDetailValues {
                request_id,
                session_epoch,
                tab_id,
                detail,
                file_path: None,
                file_content: None,
                error,
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn read_skill_file(
        &mut self,
        idx: usize,
        state: &AppState,
        skill_id: String,
        generation: String,
        path: String,
        request_id: String,
        session_epoch: u64,
        tab_id: String,
    ) {
        let result = registry_for(state).read_companion_text(&skill_id, &generation, &path);

        let (file_content, error) = match result {
            Ok(content) => (Some(content), None),
            Err(error) => (None, Some(error.to_string())),
        };
        self.send_to(
            idx,
            DaemonEvent::SkillDetailValues {
                request_id,
                session_epoch,
                tab_id,
                detail: None,
                file_path: Some(path),
                file_content,
                error,
            },
        );
    }

    pub(super) fn spawn_skill_mutation(
        &mut self,
        idx: usize,
        state: &AppState,
        request: ClientRequest,
        handle: &tokio::runtime::Handle,
    ) {
        // A combined update is pinned to the initiating foreground chat and
        // must not write to disk if that chat/loaded source is already stale.
        if let ClientRequest::UpdateSkill {
            reload_after_save: true,
            target_session_id,
            skill_id,
            name,
            request_id,
            session_epoch,
            tab_id,
            ..
        } = &request
        {
            let session = state.rest.fg().session.as_ref();
            let valid = session.is_some_and(|session| {
                target_session_id.as_ref() == Some(&session.id)
                    && session
                        .skills
                        .get(name)
                        .is_some_and(|skill| &skill.skill_id == skill_id)
                    && state.rest.fg().active_skills.contains_key(name)
            });
            if !valid {
                self.send_to(idx, DaemonEvent::SkillOp {
                    request_id: request_id.clone(),
                    session_epoch: *session_epoch,
                    tab_id: tab_id.clone(),
                    operation: "update-reload".to_string(),
                    outcomes: vec![SkillItemOutcome {
                        name: name.clone(),
                        status: "failed".to_string(),
                        error: Some("This skill is no longer loaded in the initiating chat; nothing was saved".to_string()),
                    }],
                    loaded_skill_names: loaded_names(state),
                });
                return;
            }
        }
        let client_id = self.clients[idx].id;
        let tx = self.store_tx.clone();
        let workdir = state
            .rest
            .fg()
            .session
            .as_ref()
            .map(crate::model::session::Session::workdir);
        let extra_roots = state.rest.config.extra_skill_roots.clone();
        handle.spawn_blocking(move || {
            let reply = run_skill_mutation(request, workdir, extra_roots, client_id);
            let _ = tx.send(reply);
        });
    }

    pub(super) fn reload_skills(
        &mut self,
        idx: usize,
        state: &mut AppState,
        names: Vec<String>,
        request_id: String,
        session_epoch: u64,
        tab_id: String,
    ) {
        let session_index = state.rest.foreground;
        // Explicit Reload from disk must observe edits made outside the GUI.
        // The session's discovery snapshot is otherwise only refreshed by a
        // broader rebuild; rescanning the sidebar alone does not replace it.
        if state.rest.sessions[session_index].session.is_some() {
            let fresh = registry_for(state);
            if let Some(session) = state.rest.sessions[session_index].session.as_mut() {
                session.skills = fresh;
            }
        }
        let mut outcomes = Vec::with_capacity(names.len());
        for name in names {
            if state.rest.sessions[session_index].session.is_none() {
                outcomes.push(SkillItemOutcome {
                    name,
                    status: "failed".to_string(),
                    error: Some("Open a chat to reload skills".to_string()),
                });
                continue;
            }
            let previous = state.rest.sessions[session_index]
                .active_skills
                .remove(&name);
            match crate::app::runtime::commands::skill_cmd::activate_skill(
                state,
                session_index,
                &name,
            ) {
                Ok(_) => outcomes.push(SkillItemOutcome {
                    name,
                    status: "success".to_string(),
                    error: None,
                }),
                Err(error) => {
                    if let Some(previous) = previous {
                        state.rest.sessions[session_index]
                            .active_skills
                            .insert(name.clone(), previous);
                    }
                    outcomes.push(SkillItemOutcome {
                        name,
                        status: "failed".to_string(),
                        error: Some(error.to_string()),
                    });
                }
            }
        }
        self.send_to(
            idx,
            DaemonEvent::SkillOp {
                request_id,
                session_epoch,
                tab_id,
                operation: "reload".to_string(),
                outcomes,
                loaded_skill_names: loaded_names(state),
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn finish_skill_mutation(
        &mut self,
        state: &mut AppState,
        client_id: u64,
        request_id: String,
        session_epoch: u64,
        tab_id: String,
        operation: String,
        mut outcomes: Vec<SkillItemOutcome>,
        reload_target: Option<(String, String, String, String)>,
        affected_project: Option<std::path::PathBuf>,
    ) {
        let changes_filesystem = operation != "download";
        if changes_filesystem && outcomes.iter().any(|outcome| outcome.status == "success") {
            for runtime in &mut state.rest.sessions {
                let Some(session) = runtime.session.as_mut() else {
                    continue;
                };
                let affected = affected_project.as_ref().is_none_or(|project| {
                    std::fs::canonicalize(session.workdir()).unwrap_or_else(|_| session.workdir())
                        == *project
                });
                if affected {
                    session.rebuild_system_with(
                        &crate::model::agent_def::load_registry(Some(&session.path)),
                        &state.rest.config,
                    );
                }
            }
            self.force_resync = true;
        }
        if operation == "update-reload" && outcomes.iter().any(|item| item.status == "success") {
            let replacement = reload_target.as_ref().and_then(|target| {
                let foreground = state.rest.fg();
                let session = foreground.session.as_ref()?;
                saved_skill_replacement(
                    target,
                    &session.id,
                    &session.skills,
                    foreground.active_skills.contains_key(&target.2),
                )
            });
            if let Some((name, active)) = replacement {
                // All checks happened before the single on-loop replacement.
                state.rest.fg_mut().active_skills.insert(name, active);
            } else {
                outcomes[0].status = "partial".to_string();
                outcomes[0].error = Some("Saved on disk, but the current chat was not reloaded (session, loaded skill, or saved version changed). Verify the disk version before retrying Reload from disk.".to_string());
            }
        }
        if let Some(idx) = self
            .clients
            .iter()
            .position(|client| client.id == client_id)
        {
            self.send_to(
                idx,
                DaemonEvent::SkillOp {
                    request_id: request_id.clone(),
                    session_epoch,
                    tab_id,
                    operation: operation.clone(),
                    outcomes,
                    loaded_skill_names: loaded_names(state),
                },
            );
            if changes_filesystem {
                self.list_skills(idx, state, request_id, session_epoch);
            }
        }
    }

    pub(super) fn set_skills_loaded(
        &mut self,
        idx: usize,
        state: &mut AppState,
        request: ClientRequest,
    ) {
        let ClientRequest::SetSkillsLoaded {
            names,
            loaded,
            request_id,
            session_epoch,
            tab_id,
        } = request
        else {
            return;
        };

        let session_index = state.rest.foreground;
        let has_session = state.rest.sessions[session_index].session.is_some();
        let mut outcomes = Vec::with_capacity(names.len());
        for name in names {
            if !has_session {
                outcomes.push(SkillItemOutcome {
                    name,
                    status: "failed".to_string(),
                    error: Some("Open a chat to load skills".to_string()),
                });
                continue;
            }
            let active = state.rest.sessions[session_index]
                .active_skills
                .contains_key(&name);
            if loaded && active {
                outcomes.push(SkillItemOutcome {
                    name,
                    status: "skipped".to_string(),
                    error: Some("Skill is already loaded".to_string()),
                });
            } else if !loaded && !active {
                outcomes.push(SkillItemOutcome {
                    name,
                    status: "skipped".to_string(),
                    error: Some("Skill is not loaded".to_string()),
                });
            } else if loaded {
                match crate::app::runtime::commands::skill_cmd::activate_skill(
                    state,
                    session_index,
                    &name,
                ) {
                    Ok(_) => outcomes.push(SkillItemOutcome {
                        name,
                        status: "success".to_string(),
                        error: None,
                    }),
                    Err(error) => outcomes.push(SkillItemOutcome {
                        name,
                        status: "failed".to_string(),
                        error: Some(error.to_string()),
                    }),
                }
            } else {
                crate::app::runtime::commands::skill_cmd::deactivate_skill(
                    state,
                    session_index,
                    &name,
                );
                outcomes.push(SkillItemOutcome {
                    name,
                    status: "success".to_string(),
                    error: None,
                });
            }
        }

        self.send_to(
            idx,
            DaemonEvent::SkillOp {
                request_id,
                session_epoch,
                tab_id,
                operation: if loaded { "load" } else { "unload" }.to_string(),
                outcomes,
                loaded_skill_names: loaded_names(state),
            },
        );
    }
}

/// Prepare a replacement without changing the loaded body. A later session
/// switch, manual Unload, winner change, or second disk edit makes it unsafe.
fn saved_skill_replacement(
    target: &(String, String, String, String),
    current_session_id: &str,
    registry: &SkillRegistry,
    still_loaded: bool,
) -> Option<(String, crate::app::state::ActiveSkill)> {
    let (session_id, skill_id, name, generation) = target;
    if current_session_id != session_id || !still_loaded {
        return None;
    }
    let fresh = registry.get(name)?;
    if &fresh.skill_id != skill_id || &fresh.generation != generation {
        return None;
    }
    Some((
        name.clone(),
        crate::app::state::ActiveSkill {
            body: fresh.body.clone(),
            skill_dir: fresh.skill_dir.clone(),
        },
    ))
}

fn run_skill_mutation(
    request: ClientRequest,
    workdir: Option<std::path::PathBuf>,
    extra_roots: Vec<std::path::PathBuf>,
    client_id: u64,
) -> StoreReply {
    use crate::model::skill::{
        create_owned_skill, delete_owned_skill, duplicate_to_owned, export_owned_skill_zip,
        install_owned_skill_zip, update_owned_skill_with_generation, OwnedSkillTarget, SkillEdit,
    };

    let project_identity = workdir
        .as_ref()
        .map(|path| std::fs::canonicalize(path).unwrap_or_else(|_| path.clone()));
    let registry = || SkillRegistry::load(workdir.as_deref(), &extra_roots);
    let outcome = |name: String, result: anyhow::Result<std::path::PathBuf>| SkillItemOutcome {
        name,
        status: if result.is_ok() { "success" } else { "failed" }.to_string(),
        error: result.err().map(|error| error.to_string()),
    };

    let mut saved_generation = None;
    let reload_intent = match &request {
        ClientRequest::UpdateSkill {
            reload_after_save: true,
            target_session_id: Some(session_id),
            skill_id,
            name,
            ..
        } => Some((session_id.clone(), skill_id.clone(), name.clone())),
        _ => None,
    };
    let (request_id, session_epoch, tab_id, operation, outcomes, affected_project) = match request {
        ClientRequest::CreateSkill {
            target,
            name,
            description,
            triggers,
            allowed_tools,
            instruction,
            request_id,
            session_epoch,
            tab_id,
        } => {
            let parsed = OwnedSkillTarget::parse(&target);
            let affected = parsed.as_ref().ok().and_then(|target| match target {
                OwnedSkillTarget::Global => None,
                OwnedSkillTarget::Project => project_identity.clone(),
            });
            let result = parsed.and_then(|target| {
                create_owned_skill(
                    workdir.as_deref(),
                    target,
                    &name,
                    &SkillEdit {
                        description,
                        triggers,
                        allowed_tools,
                        instruction,
                    },
                )
            });
            (
                request_id,
                session_epoch,
                tab_id,
                "create".to_string(),
                vec![outcome(name, result)],
                affected,
            )
        }
        ClientRequest::InstallSkillZip {
            target,
            name,
            data_b64,
            request_id,
            session_epoch,
            tab_id,
        } => {
            let parsed = OwnedSkillTarget::parse(&target);
            let affected = parsed.as_ref().ok().and_then(|target| match target {
                OwnedSkillTarget::Global => None,
                OwnedSkillTarget::Project => project_identity.clone(),
            });
            let result = parsed.and_then(|target| {
                use base64::Engine;
                if data_b64.len() > 96 * 1024 * 1024 {
                    anyhow::bail!("ZIP upload payload is too large");
                }
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data_b64.as_bytes())
                    .map_err(|error| anyhow::anyhow!("Invalid ZIP upload data: {error}"))?;
                install_owned_skill_zip(workdir.as_deref(), target, &name, &bytes)
            });
            (
                request_id,
                session_epoch,
                tab_id,
                "install-zip".to_string(),
                vec![outcome(name, result)],
                affected,
            )
        }
        ClientRequest::DownloadSkillZip {
            skill_id,
            generation,
            save_path,
            request_id,
            session_epoch,
            tab_id,
        } => {
            let registry = registry();
            let result = export_owned_skill_zip(
                &registry,
                &skill_id,
                &generation,
                std::path::Path::new(&save_path),
            );
            (
                request_id,
                session_epoch,
                tab_id,
                "download".to_string(),
                vec![outcome(save_path, result)],
                None,
            )
        }
        ClientRequest::UpdateSkill {
            skill_id,
            generation,
            name,
            description,
            triggers,
            allowed_tools,
            instruction,
            reload_after_save,
            request_id,
            session_epoch,
            tab_id,
            ..
        } => {
            let registry = registry();
            let affected =
                registry
                    .get_by_identity(&skill_id)
                    .and_then(|skill| match skill.source {
                        crate::model::skill::SkillSource::Global => None,
                        crate::model::skill::SkillSource::ProjectAgent
                        | crate::model::skill::SkillSource::ProjectAgents => {
                            project_identity.clone()
                        }
                        _ => None,
                    });
            let result = update_owned_skill_with_generation(
                &registry,
                &skill_id,
                &generation,
                &SkillEdit {
                    description,
                    triggers,
                    allowed_tools,
                    instruction,
                },
            );
            if reload_after_save {
                saved_generation = result
                    .as_ref()
                    .ok()
                    .map(|(_, generation)| generation.clone());
            }
            let result = result.map(|(path, _)| path);
            (
                request_id,
                session_epoch,
                tab_id,
                if reload_after_save {
                    "update-reload"
                } else {
                    "update"
                }
                .to_string(),
                vec![outcome(name, result)],
                affected,
            )
        }
        ClientRequest::DuplicateSkills {
            target,
            items,
            request_id,
            session_epoch,
            tab_id,
        } => {
            let parsed = OwnedSkillTarget::parse(&target);
            let affected = parsed.as_ref().ok().and_then(|target| match target {
                OwnedSkillTarget::Global => None,
                OwnedSkillTarget::Project => project_identity.clone(),
            });
            let registry = registry();
            let outcomes = items
                .into_iter()
                .map(|item| {
                    let result = parsed
                        .as_ref()
                        .map_err(|error| anyhow::anyhow!(error.to_string()))
                        .and_then(|target| {
                            duplicate_to_owned(
                                &registry,
                                &item.skill_id,
                                &item.generation,
                                workdir.as_deref(),
                                *target,
                                &item.destination_name,
                            )
                        });
                    outcome(item.name, result)
                })
                .collect();
            (
                request_id,
                session_epoch,
                tab_id,
                "duplicate".to_string(),
                outcomes,
                affected,
            )
        }
        ClientRequest::DeleteSkills {
            items,
            request_id,
            session_epoch,
            tab_id,
        } => {
            let registry = registry();
            let any_global = items.iter().any(|item| {
                registry
                    .get_by_identity(&item.skill_id)
                    .is_some_and(|skill| skill.source == crate::model::skill::SkillSource::Global)
            });
            let affected = if any_global {
                None
            } else {
                project_identity.clone()
            };
            let outcomes = items
                .into_iter()
                .map(|item| {
                    outcome(
                        item.name,
                        delete_owned_skill(&registry, &item.skill_id, &item.generation),
                    )
                })
                .collect();
            (
                request_id,
                session_epoch,
                tab_id,
                "delete".to_string(),
                outcomes,
                affected,
            )
        }
        _ => (
            String::new(),
            0,
            String::new(),
            "unknown".to_string(),
            vec![SkillItemOutcome {
                name: String::new(),
                status: "failed".to_string(),
                error: Some("Unsupported skill mutation".to_string()),
            }],
            project_identity,
        ),
    };

    let reload_target = reload_intent.and_then(|(session_id, skill_id, name)| {
        if !outcomes.iter().any(|outcome| outcome.status == "success") {
            return None;
        }
        Some((session_id, skill_id, name, saved_generation?))
    });
    StoreReply::SkillMutation {
        client_id,
        request_id,
        session_epoch,
        tab_id,
        operation,
        outcomes,
        reload_target,
        affected_project,
    }
}

#[cfg(test)]
mod combined_update_tests {
    use super::*;

    #[test]
    fn standalone_reload_reads_external_disk_edit_and_preserves_old_body_on_failure() {
        use crate::app::mode::Mode;
        use crate::app::runtime::event_loop::daemon::hub::core::HubInbound;
        use crate::model::{conversation::Conversation, session::Session, settings::Settings};

        let name = format!("review-{}", uuid::Uuid::new_v4());
        let project =
            std::env::temp_dir().join(format!("koma-disk-reload-{}", uuid::Uuid::new_v4()));
        let dir = project.join(".agents/skills").join(&name);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("SKILL.md");
        std::fs::write(&path, "---\ndescription: Initial\n---\nOld body\n").unwrap();
        let mut state = AppState::new(Mode::Chat);
        let settings = Settings {
            workdir: vec![project.to_string_lossy().to_string()],
            ..Default::default()
        };
        let mut session = Session::new(
            "chat-one".into(),
            project.join("session.json"),
            "test".into(),
            settings,
            Conversation::from_messages(vec![]),
        );
        session.skills = SkillRegistry::load(Some(&project), &[]);
        state.rest.sessions[0].session = Some(session);
        let original = state.rest.sessions[0]
            .session
            .as_ref()
            .unwrap()
            .skills
            .get(&name)
            .unwrap()
            .clone();
        state.rest.sessions[0].active_skills.insert(
            name.clone(),
            crate::app::state::ActiveSkill {
                body: original.body.clone(),
                skill_dir: original.skill_dir.clone(),
            },
        );
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (mut hub, _inbound) = DaemonHub::new();
        let mut client = None;
        let (frame_tx, frame_rx) = std::sync::mpsc::channel();
        hub.handle_inbound(
            HubInbound::Register {
                client_id: 31,
                frame_tx,
            },
            &mut state,
            &mut client,
            runtime.handle(),
        );

        std::fs::write(
            &path,
            "---\ndescription: Changed outside Koma\n---\nNew body\n",
        )
        .unwrap();
        hub.reload_skills(
            0,
            &mut state,
            vec![name.clone()],
            "reload-1".into(),
            7,
            "tab".into(),
        );
        let frame = frame_rx.try_recv().unwrap();
        assert!(
            matches!(frame.event, DaemonEvent::SkillOp { outcomes, .. } if outcomes[0].status == "success")
        );
        assert!(state.rest.sessions[0].active_skills[&name]
            .body
            .contains("New body"));

        std::fs::remove_file(&path).unwrap();
        hub.reload_skills(
            0,
            &mut state,
            vec![name.clone()],
            "reload-2".into(),
            7,
            "tab".into(),
        );
        let frame = frame_rx.try_recv().unwrap();
        assert!(
            matches!(frame.event, DaemonEvent::SkillOp { outcomes, .. } if outcomes[0].status == "failed")
        );
        assert!(state.rest.sessions[0].active_skills[&name]
            .body
            .contains("New body"));
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn combined_update_returns_saved_version_and_never_reloads_a_different_chat_or_generation() {
        let name = format!("review-{}", uuid::Uuid::new_v4());
        let project = std::env::temp_dir().join(format!("koma-combined-{}", uuid::Uuid::new_v4()));
        let dir = project.join(".agents/skills").join(&name);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("SKILL.md");
        std::fs::write(&path, "---\ndescription: Before\n---\nBefore body\n").unwrap();
        let before = SkillRegistry::load(Some(&project), &[])
            .get(&name)
            .unwrap()
            .clone();
        let old_generation = before.generation.clone();
        let req = ClientRequest::UpdateSkill {
            skill_id: before.skill_id.clone(),
            generation: before.generation,
            name: name.clone(),
            description: "After".into(),
            triggers: String::new(),
            allowed_tools: vec![],
            instruction: "After body".into(),
            reload_after_save: true,
            target_session_id: Some("chat-one".into()),
            request_id: "save-reload-one".into(),
            session_epoch: 7,
            tab_id: "skill:test".into(),
        };
        let reply = run_skill_mutation(req, Some(project.clone()), vec![], 19);
        let StoreReply::SkillMutation {
            request_id,
            operation,
            outcomes,
            reload_target,
            ..
        } = reply
        else {
            panic!("expected skill mutation reply")
        };
        assert_eq!(request_id, "save-reload-one");
        assert_eq!(operation, "update-reload");
        assert_eq!(outcomes[0].status, "success");
        let target = reload_target.unwrap();
        let refreshed = SkillRegistry::load(Some(&project), &[]);
        assert_eq!(target.3, refreshed.get(&name).unwrap().generation);
        let (_, active) = saved_skill_replacement(&target, "chat-one", &refreshed, true).unwrap();
        assert!(active.body.contains("After body"));
        assert!(saved_skill_replacement(&target, "chat-two", &refreshed, true).is_none());
        assert!(saved_skill_replacement(&target, "chat-one", &refreshed, false).is_none());
        std::fs::write(&path, "---\ndescription: Changed again\n---\nOther body\n").unwrap();
        let edited_again = SkillRegistry::load(Some(&project), &[]);
        assert!(saved_skill_replacement(&target, "chat-one", &edited_again, true).is_none());
        let failed = run_skill_mutation(
            ClientRequest::UpdateSkill {
                skill_id: before.skill_id,
                generation: old_generation,
                name: name.clone(),
                description: "Should not save".into(),
                triggers: String::new(),
                allowed_tools: vec![],
                instruction: "Wrong body".into(),
                reload_after_save: true,
                target_session_id: Some("chat-one".into()),
                request_id: "stale-save".into(),
                session_epoch: 7,
                tab_id: "skill:test".into(),
            },
            Some(project.clone()),
            vec![],
            19,
        );
        let StoreReply::SkillMutation {
            outcomes,
            reload_target,
            ..
        } = failed
        else {
            panic!("expected skill mutation reply")
        };
        assert_eq!(outcomes[0].status, "failed");
        assert!(reload_target.is_none());
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("Other body"));
        std::fs::remove_dir_all(project).unwrap();
    }
}
