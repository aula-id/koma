# Shared subprocess conformance peer

Run `python3 src-core-sdk/conformance/fake_adapter.py --scenario normal`.
Python 3.8+, standard library only; executable can also be launched directly.
This is a deterministic transport test peer, NOT a daemon emulator and NOT a
reference security validator. It trusts well-formed method parameters; production
adapters must implement the full validation and byte budgets in CONTRACT.md.

Scenarios (one per process):

| scenario | behavior / client assertion |
|---|---|
| normal | all methods; running turns stay running until targeted interrupt; events precede mutation replies |
| reject | turn mutations reject with invalid_input; no automatic retries |
| partial | replacement rejects after old turn was interrupted; preserve interrupted_turn_id |
| unknown | valid turn mutation returns unavailable/delivery unknown; do not retry |
| eof | exit 3 on submit/replacement without reply; report delivery unknown |
| malformed | malformed JSON after first request; fail all pending calls |
| wrong_id | extra unknown response before correct response; do not steal correlation |
| out_of_order | hold first response, respond to second then first; send two concurrent requests (including create+state) |
| event_flood | emit 300 state events before reply; bounded stalled event consumers must fail, not hang/unboundedly grow |
| flood_output | first submit overflows; restart reconciles output and subsequent turns emit valid appends |
| flood_finish | submit emits 300 events before its reply; next state read completes the turn; polling and restart recovery remain usable |
| approval | submit parks at tool approval gate; explicitly opted-in approval uses actual pending call ID |
| bad_version | hello protocol 2; client must refuse before requests |
| stderr | >64 KiB diagnostic stream; drain independently and retain bounded tail |

| output | emit full running state followed by a revisioned UTF-8 suffix append |
| hidden | explicit state hides old turn behind a newer running record |
| fragmented | write frames one byte at a time, including split multibyte UTF-8 |
| timeout | delay submit response by two seconds; client cancellation/timeout is local |
| missing_cap | omit detach by name from hello `capabilities` and `methods`; compatibility probe must refuse |
| output_gap / bad_offset | force sequence or append-offset mismatch; clients replace state by readback |
| finish | complete the observed turn on the next state read; runnable-example smoke tests |
| bad_result | return an invalid typed mutation result; report delivery unknown, not success |
| legacy_hello | an older adapter: hello has only `capabilities` (no `methods`, `features`, `events`, `limits`); `config.*` and `sessions.info` are `unsupported_method`; results carry no `session_info`; binding errors carry no `reason`. Clients gate on `capabilities` |
| config | seeded catalogue for every `config.*` branch (below); `create`/`resume` pin the session Main to `model-main` |
| wrong_binding | normal behaviour, for the binding errors E1-E3 |

Every scenario except `legacy_hello` advertises the methods, features, events and
limits it implements in hello, routes `config.*` and `sessions.info`, and answers
calls in the wrong binding with E1-E3 (`reason`, `binding`, `allowed`). `state`,
`create` and `resume` results carry `session_info` beside `state`; state events
never do. Room methods are not implemented, so, as in the real adapter, their
names answer `unsupported_method`. Unbound `detach` keeps its v1 error
(`invalid_state`, `no session bound`, no added keys) and exits.

Config catalogue. `config.json` is in memory: it starts `file:"missing"` with
revision `absent`, and `configure` writes into it (provider `prov-<name>`, model
`model-<role>`). The revision is the first 16 hex chars of sha256 over the
catalogue, so it changes on every committed write. Secret values are stored but
never projected; tests assert they never appear on stdout. The `config` scenario
starts with:

| entry | purpose |
|---|---|
| provider `prov-local` | key set; endpoint with userinfo, query and fragment (`endpoint_redacted:true`); owns `model-main` and `model-planner` |
| provider `prov-ext` | extension-managed, no key: delete gives `managed` |
| oauth `oauth-1` | signed in, owns `model-oauth` (`awareness`) |
| `model-main` | global Main |
| `model-planner` | `planner` plus an undisclosed role (`other_roles:true`) |
| `model-orphan` | provider missing (`provider_kind:"missing"`) |
| `model-unverified` | its delete commits and answers `unavailable`/`unverified`, `delivery:"unknown"`, `written:true`; unbound the fake stays up, coding it exits after the reply |

Rules the fake enforces, in the real adapter's order: coding-only methods check
the binding first; `config.get` validates `view`, which decides the binding,
then checks it; `config.roles.unassign` validates `role`, `scope` and
`if_revision`, then for session scope refuses any role but `main`
(`out_of_range`), then `if_revision` (`conflicting_params`), and only then checks
the binding; mutations then check `stale`
(`if_revision` against the current revision), then `not_found`/`managed`;
`dry_run` returns the computed effect with the current view and empty
`propagation`; `config.models.delete` with any `scope` other than `"global"` is
`out_of_range`; session-scope `config.roles.unassign` clears the Main pin (effort
resets to `default`). An unassigned `compactor`, `awareness` or `planner` reports
`origin:"inherit_main"` with Main's route and `model_uuid:null`; an unassigned
`safeguard` never inherits Main and reports `origin:"default"`,
`reason:"unassigned"`. An adapter-side delete always reports `rebind:"done"`. `config.effort.get{wait_ms:0}`
answers `loading` until options for that Main were fetched once with `wait_ms > 0`
or by `config.effort.set`; options are `default`, `low`, `medium`, `high`, or
`unsupported` with only `default` when no Main exists. `config.limits.set`
updates `session_info` (new `observed_at_ms`) without a state event or a revision
bump. CODING mutation results carry `session_info`.

Tests impose their own finite timeout. The timeout scenario intentionally sleeps.
The out_of_order scenario deliberately holds every odd response until a second
request; do not sequentially await the first response. Detach always replies and
exits immediately. Events are full copies; seq is monotonic per process.

Representative `.jsonl` fixtures in ../protocol/fixtures contain only wire frames.
Request/response IDs tie methods to results; schema validation does not replace
cross-frame assertions. Run `python3 src-core-sdk/conformance/test_fake_adapter.py`
for stdlib smoke tests. If jsonschema is installed these also validate every
`*.jsonl` file in that directory, whichever files exist.

End-to-end smoke: `python3 src-core-sdk/conformance/smoke_config.py <path to koma>`
drives the real adapter through the config methods. It needs a debug build
(`target/debug/koma`), because only debug builds honour `KOMA_TEST_DATA_DIR`; it
runs with a fresh temporary data directory, `KOMA_TELEMETRY=0` and
`KOMA_TEST_OFFLINE=1` (no update check), makes no model call, keeps the session
daemon's catalogue refresh off the network (a fresh, empty catalogue cache, the OAuth
catalogue and HTTP proxies pointed at a closed loopback port, and a check that
no log names a public catalogue host), validates every
frame against `protocol/schema.json` when jsonschema is installed, and fails if
the planted provider key appears in the adapter's output. It stops the session
daemon it started and removes its data directory.
