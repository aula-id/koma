//! IPC layer: the wire protocol the koma-daemon and its thin TUI client speak.
//!
//! The end-state architecture is ALWAYS-CLIENT/SERVER: a headless `koma-daemon`
//! owns the agent runtime + session locks, and the TUI is a thin attach/detach
//! client over a unix socket (`~/.koma/daemon.sock`) using length-prefixed JSON
//! frames. This module is STAGE 1 of that split: it defines ONLY the message
//! vocabulary ([`proto`]) — the request/response/snapshot/delta types — with no
//! transport and no callers yet. The socket server, framing, and snapshot/delta
//! emission land in later stages.
//!
//! See [`proto`] for the protocol types and the critique fixes (stable session
//! UUIDs, monotonic seq, frame-size cap) that are designed into them from the
//! start to prevent silent stream corruption later.
//!
//! STAGE 2 adds the transport primitives — [`frame`] (the shared length-prefixed
//! codec), [`server`] (bind = liveness oracle), and [`client`] (connect + frame
//! helpers) — plus a [`selftest`] that round-trips a real frame end-to-end. The
//! daemon/client loop wiring that consumes them is still a later stage; the
//! transport is additive and does not touch the TUI path.

/// Platform IPC transport aliases. On unix these are the tokio unix-domain-socket
/// types; on windows the [`win`] named-pipe backend provides the same shapes.
#[cfg(unix)]
pub type IpcListener = tokio::net::UnixListener;
#[cfg(unix)]
pub type IpcStream = tokio::net::UnixStream;
/// Blocking (std) counterpart used by the sync management/probe clients.
#[cfg(unix)]
pub type SyncIpcStream = std::os::unix::net::UnixStream;

/// Owned read/write halves of a split [`IpcStream`]. On unix the tokio unix-socket
/// owned halves (`into_split`); on windows the `tokio::io::split` halves of the
/// named-pipe stream. Consumed by the per-client connection tasks (daemon + client
/// bridge + ext host) that read and write the same stream from independent tasks.
#[cfg(unix)]
pub type IpcReadHalf = tokio::net::unix::OwnedReadHalf;
#[cfg(unix)]
pub type IpcWriteHalf = tokio::net::unix::OwnedWriteHalf;

/// Split an [`IpcStream`] into independent owned read/write halves. On unix this is
/// exactly `UnixStream::into_split()`; on windows it is `tokio::io::split`. A single
/// cross-platform shim so the read/write-task code stays identical on both.
#[cfg(unix)]
pub fn split_stream(stream: IpcStream) -> (IpcReadHalf, IpcWriteHalf) {
    stream.into_split()
}

// Windows named-pipe backend: the same IpcListener/IpcStream/SyncIpcStream shapes over
// `tokio::net::windows::named_pipe`, DACL-hardened. See [`win`].
#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::{IpcListener, IpcStream, SyncIpcStream};
#[cfg(windows)]
pub type IpcReadHalf = tokio::io::ReadHalf<IpcStream>;
#[cfg(windows)]
pub type IpcWriteHalf = tokio::io::WriteHalf<IpcStream>;
#[cfg(windows)]
pub fn split_stream(stream: IpcStream) -> (IpcReadHalf, IpcWriteHalf) {
    tokio::io::split(stream)
}

pub mod client;
pub mod conn;
pub mod frame;
#[cfg(feature = "linker")]
pub mod linker_proto;
pub mod mcp_proto;
pub mod oauth_proto;
pub mod proto;
pub mod selftest;
pub mod server;
pub mod snapshot;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod roundtrip_tests {
    //! Serde round-trip coverage for the wire protocol.
    //!
    //! Asserts that at least one of each [`proto::ClientRequest`] variant and each
    //! [`proto::DaemonFrame`] event kind (Snapshot / Delta / Ack / Error) survives
    //! a `serde_json` encode->decode unchanged. This both proves the types are
    //! wire-stable and gives the otherwise-dead scaffolding a real use.

    use super::proto::*;

    /// Encode -> decode through JSON and assert structural equality.
    fn roundtrip<T>(value: &T)
    where
        T: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug + PartialEq,
    {
        let json = serde_json::to_string(value).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(value, &back, "round-trip mismatch for {value:?}");
    }

    fn sample_session_snapshot() -> SessionSnapshot {
        use crate::dto::chat::{ChatMessage, Role};
        SessionSnapshot {
            id: "11111111-1111-4111-8111-111111111111".to_string(),
            name: "demo".to_string(),
            cwd: "/work/demo".to_string(),
            messages: vec![ChatMessage::new(Role::User, "hi")],
            // Index-aligned with `messages`; a populated entry proves the
            // display-only reasoning side-channel survives the round-trip.
            committed_reasoning: vec![Some("pondering".to_string())],
            streaming: Some("partial".to_string()),
            stream_reasoning: "thinking".to_string(),
            tokens_in: 100,
            tokens_out: 42,
            cost: 0.0012,
            tokens_cached: 16,
            waiting: true,
            awaiting_approval: false,
            approval_reason: None,
            pending_tool_calls: vec![crate::dto::chat::ToolCall {
                id: "call_1".to_string(),
                kind: "function".to_string(),
                function: crate::dto::chat::FunctionCall {
                    name: "bash".to_string(),
                    arguments: r#"{"command":"ls"}"#.to_string(),
                },
            }],
            tool_idx: 0,
            working: true,
            finished_unseen: false,
            subagents: vec![SubAgentSnapshot {
                id: 1,
                name: "explorer".to_string(),
                label: "scan the repo".to_string(),
                status: "running".to_string(),
                detached: false,
                steps: 3,
                transcript: vec!["scanned src/".to_string()],
                messages: vec![ChatMessage::new(Role::User, "scan")],
                live_text: "streaming report…".to_string(),
                committed_reasoning: Vec::new(),
            }],
            pending_subagents: vec![PendingSubagentSnapshot {
                id: 2,
                agent_name: "reviewer".to_string(),
                prompt: "review the diff".to_string(),
            }],
            // Non-empty so the round-trip proves the projected model id survives
            // serialize -> deserialize (an empty string would alias the default).
            resolved_model_id: "anthropic/claude-sonnet-4-5".to_string(),
            pending_steer: Vec::new(),
            bash_jobs: vec![],
            file_changes: vec![],
            plan_todos: vec![
                PlanTodoSnapshot {
                    content: "wire the PLAN section".to_string(),
                    status: crate::app::mode::todo::TodoStatus::InProgress,
                    locked: false,
                },
                // Exercise the locked-rail path too so the round-trip proves the
                // new field survives serialize -> deserialize non-default.
                PlanTodoSnapshot {
                    content: "serve plan to user".to_string(),
                    status: crate::app::mode::todo::TodoStatus::Pending,
                    locked: true,
                },
            ],
            // SDLC fields: exercise non-default values so the round-trip proves
            // all five survive serialize -> deserialize (None would alias default).
            sdlc_phase: Some("execute".to_string()),
            sdlc_goal: Some("ship the SDLC isolation feature".to_string()),
            sdlc_branch: Some("sdlc/isolation".to_string()),
            sdlc_open: Some(5),
            sdlc_sealed: Some(3),
        }
    }

    fn sample_global_snapshot() -> GlobalSnapshot {
        GlobalSnapshot {
            input: "type here".to_string(),
            cursor: 4,
            scroll: 0,
            follow: true,
            status: "ready".to_string(),
            work_elapsed_ms: Some(1500),
            // Non-default theme/accent so the round-trip proves a Light, non-green
            // daemon's palette tokens survive serialize -> deserialize (a Dark/green
            // pair would alias the struct default and hide a dropped field).
            theme: "light".to_string(),
            accent: "cyan".to_string(),
            // Non-default palette so the round-trip proves it survives (de)serialize.
            palette: "light".to_string(),
            // Use a populated stage-2 payload (KeyInput) so a full mode projection
            // gets round-trip coverage, not just the unit/struct-light variants.
            mode: ModeSnapshot::KeyInput(KeyInputSnapshot {
                step: 1,
                field: 0,
                endpoint: "https://openrouter.ai/api/v1".to_string(),
                api_key: "sk-test".to_string(),
                model: "openai/gpt-4o-mini".to_string(),
                query: "gpt".to_string(),
                result_sel: 2,
                first_run: true,
                from_picker: false,
            }),
            toast: Some(("info".to_string(), "saved".to_string())),
            models_cache: None,
            models_cache_endpoint: None,
            models_cache_failed: None,
            // GUI config catalogue projections default to empty here (their round-trip
            // is exercised via the config-setter paths, not this global-snapshot sample).
            providers: Vec::new(),
            config_models: Vec::new(),
            session_models: Vec::new(),
            mcp_servers: Vec::new(),
            oauth_conn_uuids: Vec::new(),
            // Non-default sub-agent viewer + `$` panel state so the round-trip proves
            // these stage-3 global flags survive serialize -> deserialize.
            agent_viewer: Some(1),
            agent_viewer_scroll: 7,
            agent_viewer_follow: false,
            subagents_open: true,
            subagent_sel: 2,
            // Non-default `@`/`/` picker selection index so the round-trip proves
            // `palette_sel` survives serialize -> deserialize (0 would alias the default).
            palette_sel: 3,
            // A staged attachment + a populated `@`-file palette so the round-trip
            // proves both new global projections survive serialize -> deserialize.
            pending_attachments: vec![crate::dto::chat::Attachment {
                marker_n: 1,
                rel_path: "images/01-shot.png".to_string(),
                mime: "image/png".to_string(),
            }],
            file_palette: Some(vec!["src/main.rs".to_string(), "src/lib.rs".to_string()]),
            agent_mode: "auto".to_string(),
            sdlc_phase: None,
            sdlc_goal: None,
            sdlc_branch: None,
            sdlc_open: None,
            sdlc_sealed: None,
            latest_version: None,
        }
    }

    fn sample_snapshot() -> StateSnapshot {
        StateSnapshot {
            foreground_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
            sessions: vec![sample_session_snapshot()],
            global: sample_global_snapshot(),
        }
    }

    #[test]
    fn client_request_variants_roundtrip() {
        let variants = vec![
            ClientRequest::Attach {
                foreground_id: Some("abc".to_string()),
                cwd: Some("/home/u/project".to_string()),
            },
            ClientRequest::Detach,
            ClientRequest::ListSessions,
            ClientRequest::Resync,
            ClientRequest::SwitchForeground {
                session_id: "abc".to_string(),
            },
            ClientRequest::SubmitInput {
                text: "hello world".to_string(),
            },
            ClientRequest::Shell {
                cmd: "ls -la".to_string(),
            },
            ClientRequest::SendKey(KeyWire {
                code: KeyCodeWire::Char('x'),
                mods: key_mods::CONTROL,
            }),
            ClientRequest::Paste {
                text: "/home/u/shot.png".to_string(),
            },
            ClientRequest::ApproveTool { approve: true },
            ClientRequest::PlanDecision {
                decision: "compact".to_string(),
            },
            ClientRequest::NewSession {
                name: Some("scratch".to_string()),
                working_dir: Some("/tmp/x".to_string()),
            },
            ClientRequest::QuitSession {
                session_id: "abc".to_string(),
            },
            ClientRequest::QuitDaemon,
        ];
        for v in &variants {
            roundtrip(v);
        }
    }

    #[test]
    fn daemon_frame_event_kinds_roundtrip() {
        let frames = vec![
            DaemonFrame {
                seq: 1,
                event: DaemonEvent::Snapshot(Box::new(sample_snapshot())),
            },
            DaemonFrame {
                seq: 2,
                event: DaemonEvent::Delta(StateDelta::TokenAppended {
                    session_id: "abc".to_string(),
                    text: "tok".to_string(),
                }),
            },
            DaemonFrame {
                seq: 3,
                event: DaemonEvent::Ack,
            },
            DaemonFrame {
                seq: 4,
                event: DaemonEvent::Error("boom".to_string()),
            },
        ];
        for f in &frames {
            roundtrip(f);
        }
    }

    #[test]
    fn state_delta_variants_roundtrip() {
        let deltas = vec![
            StateDelta::TokenAppended {
                session_id: "s".to_string(),
                text: "t".to_string(),
            },
            StateDelta::ReasoningAppended {
                session_id: "s".to_string(),
                text: "r".to_string(),
            },
            StateDelta::StatusChanged {
                session_id: Some("s".to_string()),
                text: "working".to_string(),
            },
            StateDelta::StatusChanged {
                session_id: None,
                text: "global".to_string(),
            },
            StateDelta::InputChanged {
                text: "hi".to_string(),
                cursor: 2,
            },
            StateDelta::ScrollChanged {
                scroll: 7,
                follow: false,
            },
            StateDelta::SessionStatusChanged {
                session_id: "s".to_string(),
                working: false,
                finished_unseen: true,
            },
            StateDelta::ForegroundChanged {
                session_id: "s".to_string(),
            },
            StateDelta::SessionAdded(Box::new(sample_session_snapshot())),
            StateDelta::Toast {
                kind: "error".to_string(),
                text: "nope".to_string(),
            },
        ];
        for d in &deltas {
            roundtrip(d);
        }
    }

    #[test]
    fn keywire_roundtrips_through_crossterm() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        // A mapped key with modifiers survives KeyEvent -> KeyWire -> JSON ->
        // KeyWire -> KeyEvent exactly.
        let ev = KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        let wire = KeyWire::from(ev);
        let json = serde_json::to_string(&wire).expect("serialize");
        let back: KeyWire = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(wire, back);
        let rebuilt = back.to_key_event();
        assert_eq!(rebuilt.code, KeyCode::Char('a'));
        assert!(rebuilt.modifiers.contains(KeyModifiers::CONTROL));
        assert!(rebuilt.modifiers.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn stream_event_wire_projects_and_skips() {
        use crate::service::StreamEvent;
        // A turn-relevant event projects and round-trips.
        let done = StreamEventWire::from_event(&StreamEvent::Done).expect("Done projects");
        roundtrip(&done);
        let usage = StreamEventWire::from_event(&StreamEvent::Usage {
            prompt_tokens: 10,
            completion_tokens: 5,
            cached_tokens: 2,
            cost: 0.5,
        })
        .expect("Usage projects");
        roundtrip(&usage);
        // A client-local UI event is intentionally NOT transferable.
        assert!(
            StreamEventWire::from_event(&StreamEvent::EndpointsError {
                model_id: "m".to_string(),
                error: "x".to_string(),
            })
            .is_none(),
            "endpoint events must not cross the wire"
        );
    }

    #[test]
    fn max_frame_bytes_is_64_mib() {
        assert_eq!(MAX_FRAME_BYTES, 64 * 1024 * 1024);
    }

    /// Each stage-3 `ModeSnapshot` payload (the secondary full-screen views + the
    /// last filled stubs) survives serialize -> deserialize, so a remote client
    /// reconstructs the same screen the daemon projected.
    #[test]
    fn mode_snapshot_stage3_variants_roundtrip() {
        use crate::model::usage::{ModelCostRange, RangeTotals, RoleSplit, SpendBucket, UsageData};

        // Usage: nav tokens + a populated ledger projection (both views' fields).
        let usage = ModeSnapshot::Usage(Box::new(UsageSnapshot {
            view: "global".to_string(),
            range: "week".to_string(),
            metric: "tokens".to_string(),
            data: UsageData {
                totals: RangeTotals {
                    cost: 1.25,
                    tokens_in: 1000,
                    tokens_cached: 100,
                    tokens_out: 400,
                    calls: 7,
                },
                top_models: vec![ModelCostRange {
                    model_id: "openai/gpt-4o".to_string(),
                    total_cost: 1.0,
                    tokens_in: 800,
                    tokens_cached: 80,
                    tokens_out: 300,
                    call_count: 5,
                }],
                role_split: RoleSplit {
                    main_cost: 0.9,
                    main_calls: 4,
                    sub_cost: 0.35,
                    sub_calls: 3,
                },
                heatmap_buckets: vec![SpendBucket {
                    bucket_epoch: 1_700_000_000,
                    cost: 0.5,
                    tokens: 600,
                }],
                session_models: vec![],
                session_hourly: vec![],
                session_calls: 7,
            },
        }));
        roundtrip(&usage);

        // MessageRewind: newest-first entries + cursor.
        roundtrip(&ModeSnapshot::MessageRewind(RewindSnapshot {
            entries: vec![
                RewindEntrySnapshot {
                    vec_index: 4,
                    content: "latest".to_string(),
                },
                RewindEntrySnapshot {
                    vec_index: 2,
                    content: "earlier".to_string(),
                },
            ],
            selected: 1,
        }));

        // Effort: options + cursor + note.
        roundtrip(&ModeSnapshot::Effort(EffortSnapshot {
            options: vec!["default".to_string(), "high".to_string()],
            selected: 1,
            note: "model supports effort".to_string(),
        }));

        // Model command: role_pick submode + options + cursor + note.
        roundtrip(&ModeSnapshot::Model(Box::new(ModelCmdSnapshot {
            sub: "role_pick".to_string(),
            role: Some("main".to_string()),
            agent_name: None,
            options: vec![
                (None, "(inherit global)".to_string()),
                (
                    Some("uuid-1".to_string()),
                    "gpt-4o — openai/gpt-4o @ OpenAI".to_string(),
                ),
            ],
            cursor: 1,
            note: "pick a model for the main role".to_string(),
            lines: vec![],
        })));

        // Model command: help submode.
        roundtrip(&ModeSnapshot::Model(Box::new(ModelCmdSnapshot {
            sub: "help".to_string(),
            role: None,
            agent_name: None,
            options: vec![],
            cursor: 0,
            note: String::new(),
            lines: vec![
                "Usage: /model [role|agent]".to_string(),
                "  role  — pick a model for a role".to_string(),
                "  agent — pick a model for an agent".to_string(),
            ],
        })));

        // Model command: agent_pick submode.
        roundtrip(&ModeSnapshot::Model(Box::new(ModelCmdSnapshot {
            sub: "agent_pick".to_string(),
            role: None,
            agent_name: Some("explore".to_string()),
            options: vec![
                (None, "(inherit main)".to_string()),
                (
                    Some("uuid-2".to_string()),
                    "fast — gpt-4o-mini @ OpenAI".to_string(),
                ),
            ],
            cursor: 1,
            note: "pick a model for this agent".to_string(),
            lines: vec![],
        })));

        // SessionPicker: metadata list + query + filtered subset + cursor.
        roundtrip(&ModeSnapshot::SessionPicker(PickerSnapshot {
            query: "auth".to_string(),
            all: vec![SessionMetaSnapshot {
                id: "11111111-1111-4111-8111-111111111111".to_string(),
                name: "auth-refactor".to_string(),
                modified_secs: 1_700_000_500,
                message_count: 12,
                locked: false,
            }],
            filtered_idx: vec![0],
            selected: 0,
        }));

        // Agents: the largest payload — list + drafts + overlays + keyless catalogue.
        let agents = ModeSnapshot::Agents(Box::new(AgentsSnapshot {
            agents: vec![crate::ipc::proto::AgentEntry {
                name: "explore".to_string(),
                description: "scout the codebase".to_string(),
                ..crate::ipc::proto::AgentEntry::default()
            }],
            list_sel: 0,
            in_detail: true,
            mode: "edit".to_string(),
            field: "prompt".to_string(),
            editing: false,
            create_scope: "session".to_string(),
            draft_name: "explore".to_string(),
            draft_description: "scout the codebase".to_string(),
            draft_conditions: String::new(),
            draft_model_uuid: Some("model-uuid".to_string()),
            draft_model_legacy: None,
            draft_tools: "read, grep".to_string(),
            draft_body: "You are a scout.".to_string(),
            tool_picker: Some(ToolPickerSnapshot {
                options: vec!["read".to_string(), "grep".to_string()],
                checked: vec![true, false],
                cursor: 0,
                filter: String::new(),
            }),
            model_picker: Some(AgentModelPickerSnapshot {
                options: vec![(None, "(inherit main)".to_string())],
                cursor: 0,
            }),
            editor: Some((
                "prompt".to_string(),
                TextEditorSnapshot {
                    lines: vec!["You are a scout.".to_string()],
                    row: 0,
                    col: 3,
                    scroll: 0,
                },
            )),
            editor_clear_confirm: false,
            catalogue_models: vec![CatalogueModelSnapshot {
                uuid: "model-uuid".to_string(),
                name: "GPT-4o".to_string(),
                model_id: "openai/gpt-4o".to_string(),
                provider_uuid: "prov-uuid".to_string(),
            }],
            catalogue_providers: vec![CatalogueProviderSnapshot {
                uuid: "prov-uuid".to_string(),
                name: "OpenRouter".to_string(),
                endpoint: "https://openrouter.ai/api/v1".to_string(),
            }],
        }));
        roundtrip(&agents);
    }

    /// The `/settings` OAuth submenu's connect-flow projection ([`OAuthFlowSnapshot`])
    /// and one connection draft ([`OAuthDraftSnapshot`], with its `status` field) —
    /// every flow-state variant survives the wire round-trip.
    #[test]
    fn oauth_flow_and_draft_snapshot_roundtrip() {
        roundtrip(&OAuthFlowSnapshot {
            kind: "idle".to_string(),
            ..Default::default()
        });
        roundtrip(&OAuthFlowSnapshot {
            kind: "starting".to_string(),
            ..Default::default()
        });
        roundtrip(&OAuthFlowSnapshot {
            kind: "pick".to_string(),
            cursor: 2,
            ..Default::default()
        });
        roundtrip(&OAuthFlowSnapshot {
            kind: "codex_wait".to_string(),
            url: "https://auth.openai.com/oauth/authorize?...".to_string(),
            frame: 3,
            copied: true,
            ..Default::default()
        });
        roundtrip(&OAuthFlowSnapshot {
            kind: "codex_paste".to_string(),
            input: "eyJhbGciOi...".to_string(),
            ..Default::default()
        });
        roundtrip(&OAuthFlowSnapshot {
            kind: "kilo_wait".to_string(),
            url: "https://kilo.ai/device".to_string(),
            user_code: "ABCD-1234".to_string(),
            frame: 7,
            copied: true,
            ..Default::default()
        });
        roundtrip(&OAuthFlowSnapshot {
            kind: "failed".to_string(),
            error: "device login denied".to_string(),
            ..Default::default()
        });

        roundtrip(&OAuthDraftSnapshot {
            uuid: "11111111-1111-4111-8111-111111111111".to_string(),
            label: "codex (dev@example.com)".to_string(),
            provider: "codex".to_string(),
            key: "sk-fake".to_string(),
            status: "renews in 3d".to_string(),
        });
    }

    // ─── SDLC mode isolation projection tests ──────────────────────────────────
    //
    // These prove the snapshot projection mode-gates SDLC and Plan fields:
    //   • SDLC fields only appear when agent_mode == Sdlc
    //   • Plan todos only appear when agent_mode == Plan
    //   • Cross-session rails never leak (session B's projection is independent)

    use crate::app::state::AgentMode;
    use crate::model::conversation::Conversation;
    use crate::model::session::Session;
    use crate::model::settings::Settings;

    struct ProjectionFixture {
        state: crate::app::state::AppState,
        roots: Vec<std::path::PathBuf>,
    }

    impl std::ops::Deref for ProjectionFixture {
        type Target = crate::app::state::AppState;

        fn deref(&self) -> &Self::Target {
            &self.state
        }
    }

    impl std::ops::DerefMut for ProjectionFixture {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.state
        }
    }

    impl Drop for ProjectionFixture {
        fn drop(&mut self) {
            for root in &self.roots {
                let _ = std::fs::remove_dir_all(root);
            }
        }
    }

    fn projection_session(
        tag: &str,
        goal: &str,
        open: usize,
        sealed: usize,
    ) -> (Session, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "koma-ipc-projection-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mission = serde_json::json!({
            "contract_version": 2,
            "id": format!("mission-{tag}"),
            "goal": goal,
            "non_goals": [],
            "acceptance": ["projection is authoritative"],
            "lane": "standard",
            "verify_plan": [],
            "human_gates": [],
            "human_gates_approved": [],
            "risks": [],
            "worktree_name": null,
            "branch": null,
            "worktree_path": null,
            "target_worktree_path": null,
            "target_branch": null,
            "target_head": null,
            "rationale": "test fixture",
            "phase": "execute",
            "approved": false,
            "hash": "fixture",
            "graph_hash": null,
            "needs_reapproval": false,
            "amendment_note": null
        });
        std::fs::write(
            root.join("mission.json"),
            serde_json::to_vec_pretty(&mission).unwrap(),
        )
        .unwrap();
        let conn = crate::model::msglog::open(&root).unwrap();
        crate::model::sdlc::graph::ensure_tables(&conn).unwrap();
        for i in 0..open {
            conn.execute(
                "INSERT INTO sdlc_nodes (id, title, status, notes, verify_bit, updated_at, owned_paths) VALUES (?1, ?2, 'pending', '', 0, 1, '[]')",
                rusqlite::params![format!("{tag}-open-{i}"), format!("{tag} open {i}")],
            )
            .unwrap();
        }
        for i in 0..sealed {
            conn.execute(
                "INSERT INTO sdlc_nodes (id, title, status, notes, verify_bit, updated_at, owned_paths) VALUES (?1, ?2, 'done', '', 1, 1, '[]')",
                rusqlite::params![format!("{tag}-sealed-{i}"), format!("{tag} sealed {i}")],
            )
            .unwrap();
        }
        drop(conn);
        let session = Session::new(
            format!("session-{tag}"),
            root.clone(),
            "pwd".into(),
            Settings::default(),
            Conversation::from_messages(vec![]),
        );
        (session, root)
    }

    /// Build two genuinely persisted sessions. Mission goal and L2 graph counts
    /// are read by projection from each session's artifacts, never runtime caches.
    fn build_two_session_state(
        mode_a: AgentMode,
        sdlc_phase_a: Option<&str>,
        mode_b: AgentMode,
    ) -> ProjectionFixture {
        let (session_a, root_a) = projection_session("a", "goal-a", 5, 3);
        let (session_b, root_b) = projection_session("b", "goal-b", 1, 1);
        let mut rest = crate::app::state::AppStateRest::default();
        rest.sessions[0].id = "session-a".to_string();
        rest.sessions[0].session = Some(session_a);
        rest.sessions[0].agent_mode = mode_a;
        rest.sessions[0].sdlc_phase = sdlc_phase_a.map(str::to_string);
        rest.sessions[0].sdlc_branch = Some("sdlc/feat-a".to_string());
        rest.sessions[0].plan_todos = vec![crate::app::mode::todo::TodoItem {
            content: "step 1".to_string(),
            status: crate::app::mode::todo::TodoStatus::Completed,
            priority: crate::app::mode::todo::TodoPriority::Medium,
            locked: false,
        }];
        let mut rt_b = crate::app::state::SessionRuntime::new();
        rt_b.id = "session-b".to_string();
        rt_b.session = Some(session_b);
        rt_b.agent_mode = mode_b;
        rt_b.sdlc_phase = Some("prepare".into());
        rt_b.sdlc_branch = Some("sdlc/feat-b".into());
        rest.sessions.push(rt_b);
        rest.foreground = 0;
        ProjectionFixture {
            state: crate::app::state::AppState { rest },
            roots: vec![root_a, root_b],
        }
    }

    #[test]
    fn sdlc_to_auto_clears_sdlc_in_projection() {
        let state = build_two_session_state(AgentMode::Sdlc, Some("execute"), AgentMode::Auto);
        // Project session A (SDLC mode): SDLC fields should be present.
        let snap = crate::ipc::snapshot::projection::build_snapshot(&state);
        let session_a = &snap.sessions[0];
        assert_eq!(session_a.sdlc_phase.as_deref(), Some("execute"));
        assert_eq!(session_a.sdlc_branch.as_deref(), Some("sdlc/feat-a"));
        assert_eq!(session_a.sdlc_goal.as_deref(), Some("goal-a"));
        assert_eq!(session_a.sdlc_open, Some(5));
        assert_eq!(session_a.sdlc_sealed, Some(3));

        // Now change session A to Auto mode — SDLC fields must clear.
        let mut state2 = state;
        state2.rest.sessions[0].agent_mode = AgentMode::Auto;
        let snap2 = crate::ipc::snapshot::projection::build_snapshot(&state2);
        let session_a2 = &snap2.sessions[0];
        assert!(
            session_a2.sdlc_phase.is_none(),
            "SDLC phase cleared when mode=auto"
        );
        assert!(
            session_a2.sdlc_goal.is_none(),
            "SDLC goal cleared when mode=auto"
        );
        assert!(
            session_a2.sdlc_branch.is_none(),
            "SDLC branch cleared when mode=auto"
        );
        assert!(
            session_a2.sdlc_open.is_none(),
            "SDLC open cleared when mode=auto"
        );
        assert!(
            session_a2.sdlc_sealed.is_none(),
            "SDLC sealed cleared when mode=auto"
        );
    }

    #[test]
    fn sdlc_to_plan_clears_sdlc_and_preserves_plan() {
        let mut state = build_two_session_state(AgentMode::Sdlc, Some("assess"), AgentMode::Auto);
        // Session A starts in SDLC
        let snap = crate::ipc::snapshot::projection::build_snapshot(&state);
        assert!(snap.sessions[0].sdlc_phase.is_some());
        assert!(
            snap.sessions[0].plan_todos.is_empty(),
            "Plan todos empty in SDLC mode"
        );

        // Switch session A to Plan mode
        state.rest.sessions[0].agent_mode = AgentMode::Plan;
        let snap2 = crate::ipc::snapshot::projection::build_snapshot(&state);
        let sa = &snap2.sessions[0];
        assert!(sa.sdlc_phase.is_none(), "SDLC phase cleared when mode=plan");
        assert!(sa.sdlc_goal.is_none(), "SDLC goal cleared when mode=plan");
        assert_eq!(sa.plan_todos.len(), 1, "Plan todos projected in plan mode");
        assert_eq!(sa.plan_todos[0].content, "step 1");
    }

    #[test]
    fn plan_to_auto_clears_plan_and_no_sdlc_leak() {
        let mut state = build_two_session_state(AgentMode::Plan, None, AgentMode::Auto);
        // Session A in Plan mode — plan todos present, SDLC clear
        let snap = crate::ipc::snapshot::projection::build_snapshot(&state);
        assert_eq!(snap.sessions[0].plan_todos.len(), 1);
        assert!(snap.sessions[0].sdlc_phase.is_none());

        // Switch session A to Auto — plan todos must clear too
        state.rest.sessions[0].agent_mode = AgentMode::Auto;
        let snap2 = crate::ipc::snapshot::projection::build_snapshot(&state);
        assert!(
            snap2.sessions[0].plan_todos.is_empty(),
            "Plan todos cleared when mode=auto"
        );
        assert!(snap2.sessions[0].sdlc_phase.is_none());
    }

    #[test]
    fn cross_session_no_rail_leakage() {
        let mut state = build_two_session_state(AgentMode::Sdlc, Some("execute"), AgentMode::Sdlc);
        let snap = crate::ipc::snapshot::projection::build_snapshot(&state);
        let sa = &snap.sessions[0];
        let sb = &snap.sessions[1];
        assert_eq!(sa.sdlc_goal.as_deref(), Some("goal-a"));
        assert_eq!(sa.sdlc_open, Some(5));
        assert_eq!(sa.sdlc_sealed, Some(3));
        assert!(sb.sdlc_phase.is_none());
        assert!(sb.sdlc_goal.is_none());
        assert!(sb.sdlc_branch.is_none());
        assert!(sb.sdlc_open.is_none());
        assert!(sb.sdlc_sealed.is_none());
        assert!(sb.plan_todos.is_empty());

        state.rest.foreground = 1;
        let snap = crate::ipc::snapshot::projection::build_snapshot(&state);
        let sa = &snap.sessions[0];
        let sb = &snap.sessions[1];
        assert!(sa.sdlc_phase.is_none());
        assert!(sa.sdlc_goal.is_none());
        assert_eq!(sb.sdlc_phase.as_deref(), Some("prepare"));
        assert_eq!(sb.sdlc_goal.as_deref(), Some("goal-b"));
        assert_eq!(sb.sdlc_branch.as_deref(), Some("sdlc/feat-b"));
        assert_eq!(sb.sdlc_open, Some(1));
        assert_eq!(sb.sdlc_sealed, Some(1));
    }
}
