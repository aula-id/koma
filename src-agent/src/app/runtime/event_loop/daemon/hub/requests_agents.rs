//! Agent-definition mutation arm bodies for [`super::core::DaemonHub`] — the GUI
//! /agents dashboard's create / save / rename / delete. Split out of `requests.rs`
//! (like `requests_config`) so each request file stays focused + under the size cap.
//! Each method is called from `requests.rs`'s `handle_controller_mutation` match,
//! mirrors the TUI's `actions::agents` handlers (built-in protection + the
//! `rebuild_system` roster invariant), and re-pushes a fresh `AgentsValues` — the shared
//! `send_agents_values` builder in `requests_read` — as its reply, the same
//! `SetSessionPrefs` → `SettingsValues` mutation-reply framing the other GUI setters use.
//! An error surfaces as a [`DaemonEvent::Error`]; a success's re-push IS the reply.

use crate::app::state::AppState;
use crate::ipc::proto::{ClientRequest, DaemonEvent};

use super::core::DaemonHub;

impl DaemonHub {
    /// Upsert one sub-agent definition (the /agents editor's create / save / rename).
    ///
    /// **Write scope is DERIVED on an EDIT, client-chosen only on a CREATE** — mirroring the
    /// TUI exactly (`actions::agents`): `handle_create_agent` writes to the user's
    /// `create_scope`, while `handle_save_agent` derives the write tier from the edited
    /// agent's OWN `source` and ignores any UI scope. So here:
    /// - CREATE (`original_name` is `None`): the wire `scope` (`"global"` / `"session"`) is
    ///   honoured as sent, seeding from `AgentDef::default()` (a blank agent).
    /// - EDIT (`original_name` is `Some`): the wire `scope` is IGNORED. The write tier is the
    ///   existing def's source — `Global`→global, `Session`→session, `Builtin`→session (a
    ///   session override; a built-in is never mutated in place). A missing existing def (a
    ///   stale / racy edit) falls back to the wire `scope`. The seed is the existing def, so
    ///   non-editor frontmatter (steps / effort / temperature / color) round-trips, and the
    ///   legacy `model` / `provider` / `provider_uuid` slots are cleared (the editor drives
    ///   `model_uuid` now).
    ///
    /// Because an EDIT's write tier is derived from the SAME source it looked up, a non-builtin
    /// edit can never carry one tier's content into another tier's file (finding #2); the only
    /// cross-tier seed is the intended `Builtin`→session override. On a rename the OLD file is
    /// deleted from that SAME derived tier (never the request's scope) AFTER the new one lands
    /// (finding #1), so no file is orphaned. `save_agent` re-validates the name (path-safe);
    /// a successful mutation rebuilds the foreground roster (the stale-roster invariant) and
    /// re-pushes `AgentsValues`. A `"session"` target with no live session is an error.
    ///
    /// Takes the whole [`ClientRequest`] (destructured here) so its dispatch arm in
    /// `requests.rs` stays a one-line call rather than an 8-field re-bind.
    pub(super) fn set_agent(&mut self, idx: usize, state: &mut AppState, req: ClientRequest) {
        use crate::model::agent_def::{
            delete_agent as delete_agent_file, load_registry, save_agent, AgentDef, AgentScope,
            AgentSource,
        };
        // Only the `SetAgent` variant reaches here (its dispatch arm); anything else is a
        // clean no-op guard rather than a panic.
        let ClientRequest::SetAgent {
            original_name,
            scope,
            name,
            description,
            conditions,
            model_uuid,
            tools,
            prompt,
        } = req
        else {
            return;
        };

        // CREATE vs EDIT is signalled by `original_name`: `Some` = editing an existing agent
        // (scope derived), `None` = creating a fresh one (scope honoured as sent).
        let is_edit = original_name.is_some();

        // The foreground session's dir (the session-scope target + the registry overlay to
        // load the existing def / derive its source). Owned so no borrow of `state` is held.
        let session_dir = state.rest.fg().session.as_ref().map(|s| s.path.clone());

        // Load the merged registry to fetch the existing def for an EDIT — its content is the
        // seed (preserve non-editor frontmatter) AND its `source` is the derived write tier.
        // The pre-edit name (`original_name`) is what an edit / rename looks up.
        let lookup = original_name.clone().unwrap_or_else(|| name.clone());
        let registry = load_registry(session_dir.as_deref());
        let existing = registry.get(&lookup).cloned();

        // Derive the write scope (the root decision — mirror `handle_save_agent`):
        //   - CREATE: honour the wire `scope` as the user chose it.
        //   - EDIT: derive from the existing def's source tier, IGNORING the wire `scope`.
        //     Global→global, Session→session, Builtin→session override. A missing existing def
        //     (a stale / racy edit) has no source to derive from, so it falls back to the wire.
        let want_session = if is_edit {
            match existing.as_ref().map(|d| d.source) {
                Some(AgentSource::Global) => false,
                Some(AgentSource::Session) | Some(AgentSource::Builtin) => true,
                None => scope == "session",
            }
        } else {
            scope == "session"
        };
        let scope_target = match (want_session, session_dir.as_deref()) {
            (true, Some(dir)) => AgentScope::Session(dir),
            (true, None) => {
                self.send_to(
                    idx,
                    DaemonEvent::Error("no active session for a session-scoped agent".into()),
                );
                return;
            }
            (false, _) => AgentScope::Global,
        };

        // Seed the def: EDIT from the existing one so `steps` / `effort` / `temperature` /
        // `color` survive; CREATE from a blank default (mirrors `AgentsState::to_agent_def`,
        // which branches Create→default, Edit→current_agent). For a Builtin→session override
        // the seed IS the built-in def — that is exactly what an override is (finding #2). For
        // Global / Session sources the seed's tier equals the derived write tier, so no
        // cross-tier content bleed can occur. Overwrite ONLY the editor-carried fields and
        // clear the legacy model / provider slots (the editor drives `model_uuid` now).
        let mut def = if is_edit {
            existing.unwrap_or_default()
        } else {
            AgentDef::default()
        };
        def.name = name.clone();
        def.description = description.trim().to_string();
        def.conditions = conditions.trim().to_string();
        def.model_uuid = model_uuid;
        def.model = None;
        def.provider = None;
        def.provider_uuid = None;
        def.tools = tools;
        def.prompt = prompt;
        // `source` is `#[serde(skip)]` (never written to disk — the tier decides it), but set
        // it to match the derived write tier for a faithful in-memory def.
        def.source = if want_session {
            AgentSource::Session
        } else {
            AgentSource::Global
        };
        def.file_path = None;

        // Persist. On success: on a rename delete the OLD file from the SAME derived tier
        // (`scope_target`, never the request's scope) AFTER the new one landed, so a save error
        // never orphans the old def and the delete can never hit the wrong tier. A delete-old
        // failure is logged (not silent — finding #3) but does not fail the mutation. Then
        // rebuild the roster and re-push `AgentsValues`.
        match save_agent(scope_target, &def) {
            Ok(_) => {
                if let Some(orig) = original_name.as_deref() {
                    if orig != name.as_str() {
                        if let Err(e) = delete_agent_file(scope_target, orig) {
                            eprintln!("Warning: agent rename left old file {orig}: {e}");
                        }
                    }
                }
                if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                    sess.rebuild_system();
                }
                self.send_agents_values(idx, state);
            }
            Err(e) => self.send_to(idx, DaemonEvent::Error(format!("{e:#}"))),
        }
    }

    /// Delete one file-backed sub-agent definition (`<scope>/<name>.md`).
    ///
    /// Built-in protection first: a built-in has no file and is not deletable (error).
    /// Otherwise scope-resolve (`"session"` needs a live session), delete via the data
    /// layer (idempotent — a missing file is success), rebuild the foreground roster, and
    /// re-push `AgentsValues`. Deleting a session / global override that shadowed a built-in
    /// simply re-exposes the built-in on the next load.
    pub(super) fn delete_agent(
        &mut self,
        idx: usize,
        state: &mut AppState,
        scope: String,
        name: String,
    ) {
        use crate::model::agent_def::{
            delete_agent as delete_agent_file, load_registry, AgentScope, AgentSource,
        };

        let session_dir = state.rest.fg().session.as_ref().map(|s| s.path.clone());

        // Built-in protection: never delete a built-in (it has no file anyway).
        let registry = load_registry(session_dir.as_deref());
        if registry.get(&name).map(|d| d.source) == Some(AgentSource::Builtin) {
            self.send_to(
                idx,
                DaemonEvent::Error("cannot delete a built-in agent".into()),
            );
            return;
        }

        let scope_target = match (scope.as_str(), session_dir.as_deref()) {
            ("session", Some(dir)) => AgentScope::Session(dir),
            ("session", None) => {
                self.send_to(
                    idx,
                    DaemonEvent::Error("no active session for a session-scoped agent".into()),
                );
                return;
            }
            _ => AgentScope::Global,
        };

        match delete_agent_file(scope_target, &name) {
            Ok(()) => {
                if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                    sess.rebuild_system();
                }
                self.send_agents_values(idx, state);
            }
            Err(e) => self.send_to(idx, DaemonEvent::Error(format!("{e:#}"))),
        }
    }
}
