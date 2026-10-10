# Session history paging verification

History uses registry metadata only. Folder identity is the stored `pwd_hash`; no history query canonicalizes workdirs or opens transcripts. Default pages have 50 rows, with a maximum of 100, a lookahead row, and scope-checked keyset cursors. Live exclusions use an indexed temporary table, avoiding the SQL parameter limit.

Both GUI surfaces share `SessionHistoryBrowser` and retain separate host-scoped navigation state. History search requests are debounced by 200 ms. Row heights are measured; rendering uses viewport rows plus eight rows of overscan on either side and retains the focused row. TUI queries run in workers and its renderer slices rows before formatting them. Explicit action replies, rather than absence from a page, complete deletion.

## Synthetic benchmark

Command: `cargo test -p agent --no-default-features benchmark_registry_pages -- --ignored --nocapture`.

Measured in this workspace using the unoptimized Rust test build, an in-memory SQLite registry, and no live exclusions. The many-folder case has ten sessions per folder; the other case puts all sessions in one folder. Times include temporary exclusion setup and query execution, and exclude serialization; payload sizes include JSON serialization. These are single-run measurements, not production latency estimates or end-to-end speedup claims.

| Sessions | Distribution | Folder page | Session page | Folder payload | Session payload |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1,000 | One folder | 1.61 ms | 0.287 ms | 404 B | 5,791 B |
| 1,000 | Many folders | 1.78 ms | 0.132 ms | 7,028 B | 1,365 B |
| 10,000 | One folder | 12.80 ms | 0.309 ms | 406 B | 5,842 B |
| 10,000 | Many folders | 14.48 ms | 0.137 ms | 7,181 B | 1,365 B |
| 100,000 | One folder | 142.40 ms | 0.365 ms | 408 B | 5,893 B |
| 100,000 | Many folders | 139.03 ms | 0.154 ms | 7,333 B | 1,365 B |

Actual `EXPLAIN QUERY PLAN` output for folder-scoped sessions:

```text
SEARCH s USING INDEX idx_sessions_pwd_activity (pwd_hash=?)
CORRELATED SCALAR SUBQUERY 1
SEARCH l USING PRIMARY KEY (uuid=?)
```

Actual folder query plan:

```text
CO-ROUTINE grouped
SCAN s USING INDEX idx_sessions_pwd
CORRELATED SCALAR SUBQUERY 1
SEARCH l USING PRIMARY KEY (uuid=?)
SCAN grouped
USE TEMP B-TREE FOR ORDER BY
```

Folder aggregation necessarily examines registry metadata for counts and activity. SQLite does the aggregation; Rust receives only a page. The folder session query uses the activity index and does not load the rest of the folder. Before specializing the folder-scoped SQL branch, that same synthetic 100,000-session query measured 181 ms; this comparison is an implementation diagnostic, not a baseline for the previous application.

## Checks

- Registry tests cover missing paths, identical basenames, current-folder priority, equal timestamps, cursor boundaries and mismatches, large live exclusion sets, Unicode and literal search, UUID search, empty results, page limits, and concurrent insertion/rename/deletion/live changes.
- TUI state tests cover folder cursor restoration, folder deletion guards, debounce generation cancellation, append deduplication, and failure preservation.
- Remote tests cover object and bare-array compatibility, action arguments, and legacy live status parsing.
- GUI state tests cover stale/closed replies, host/surface isolation, append failure/retry, identity deduplication, and explicit action correlation.
- Chromium tests cover folder navigation, scoped search and Back, refresh failure preservation, and fewer than 35 mounted history rows with 2,000 loaded rows. A real-time idle-minute test verifies continued two-second live polling with zero additional history requests.
- Rust default-feature and headless checks and the Vite production build pass without warnings. The full Rust suite passes: 1,965 passed, three intentionally ignored.
- The full GUI logic suite passes (71 tests), the full Chromium suite passes (51 tests), and existing design model/store assertions pass.
- The full TypeScript check passes. Existing clipboard, design-editor, and skills type errors exposed by this check were corrected. New Rust implementation and test code avoids `unwrap()` and `expect()`.
- The production build uses the expression-free Lottie light player and a documented 3 MB chunk budget for large generated/editor modules; it emits no build warnings.

## Remote compatibility and limits

`koma sessions --json` retains its default `{ live, history }` shape. New callers use `--live-only`, `--query folders|sessions`, or the typed `--history-request` transport. New page replies declare protocol version 1 and paging support. Ordinary SSH, parse, and database errors do not activate compatibility fallback.

Confirmed legacy responses are cached by endpoint and key configuration, grouped and paged locally through the shared query implementation. Stored remote paths identify legacy folders because older responses lack folder hashes. Current-directory context comes from remote `pwd`; local filesystem hashing is never used for remote history. Legacy live polling uses `daemon status` without reloading history: existing names and activity indicators remain from the last full discovery, and newly discovered daemons initially display their UUID. Explicit Refresh retrieves fresh metadata. The history browser shows that server pagination needs a newer binary.

A configured real SSH host was not available for end-to-end remote testing. Compatibility parsing and transport construction are covered locally; remote network latency is not included in the benchmark. Legacy status output must have the supported `koma daemon:` shape; unexpected output produces a retryable error.
