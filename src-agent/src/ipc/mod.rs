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

pub mod client;
pub mod conn;
pub mod frame;
pub mod proto;
pub mod selftest;
pub mod server;
pub mod snapshot;

#[cfg(test)]
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
                steps: 3,
                transcript: vec!["scanned src/".to_string()],
                messages: vec![ChatMessage::new(Role::User, "scan")],
            }],
            pending_subagents: vec![PendingSubagentSnapshot {
                id: 2,
                agent_name: "reviewer".to_string(),
                prompt: "review the diff".to_string(),
            }],
            // Non-empty so the round-trip proves the projected model id survives
            // serialize -> deserialize (an empty string would alias the default).
            resolved_model_id: "anthropic/claude-sonnet-4-5".to_string(),
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
        let ev = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
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
                RewindEntrySnapshot { vec_index: 4, content: "latest".to_string() },
                RewindEntrySnapshot { vec_index: 2, content: "earlier".to_string() },
            ],
            selected: 1,
        }));

        // Effort: options + cursor + note.
        roundtrip(&ModeSnapshot::Effort(EffortSnapshot {
            options: vec!["default".to_string(), "high".to_string()],
            selected: 1,
            note: "model supports effort".to_string(),
        }));

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
}
