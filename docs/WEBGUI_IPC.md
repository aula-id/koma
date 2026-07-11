# Web GUI: Communicating with Rust

A GUI tab does not call Rust directly. It communicates through the same typed
request/push bridge used by the rest of the web GUI:

```text
React component or Zustand action
  -> req({ r: '...' })
  -> window.ipc.postMessage(JSON)
  -> wry IPC handler
  -> ClientMsg::Req(GuiReq)
  -> handle_gui_req()
  -> HostCtl or attached daemon request
  -> host computation / daemon response
  -> PushEnvelope
  -> UserEvent::Push(JSON)
  -> window.__komaClient.push(JSON)
  -> useKoma.push() reducer
  -> Zustand state
  -> tab renders
```

The relevant implementation is split between `src-webgui/` and
`src-agent/src/app/runtime/gui/` plus the host-side GUI relay in
`src-agent/src/app/runtime/client/`.

## The bridge contract

The JavaScript side has two injected globals:

| Global | Direction | Purpose |
|---|---|---|
| `window.ipc.postMessage(json)` | JS → Rust | Sends window commands and GUI requests |
| `window.__komaClient.push(json)` | Rust → JS | Delivers serialized push envelopes |

The request envelope is tagged with `t` and the request kind with `r`:

```json
{
  "t": "req",
  "r": "GitStatus"
}
```

A request with fields looks like this:

```json
{
  "t": "req",
  "r": "FileDiff",
  "path": "src/main.rs"
}
```

Rust replies asynchronously with a push envelope tagged by `k`:

```json
{
  "k": "GitStatus",
  "root": "/work/project",
  "branch": "main",
  "detached": false,
  "ahead": 0,
  "behind": 0,
  "staged": [],
  "unstaged": [],
  "error": null,
  "keyName": null,
  "inProgress": null,
  "conflicted": []
}
```

The authoritative request and push unions are declared in:

- `src-webgui/src/koma.d.ts` for `GuiReq`;
- `src-webgui/src/store/koma.ts` for `PushEnvelope`;
- `src-agent/src/app/runtime/gui/proto.rs` for Rust request deserialization;
- `src-agent/src/app/runtime/client/push_proto.rs` for Rust push serialization.

Keep these contracts synchronized. A TypeScript type alone does not make a
request work: Rust must deserialize, dispatch, process, and answer it.

## Choose the owner before choosing the protocol

A useful first decision is **who owns the data**, not which component happens to
render it. The following table describes the current choices. The rows are not
mutually exclusive: `host-local`/`daemon-backed` describe where computation runs,
while `persisted/global`/`session-scoped` describe the lifetime and authority of
the data.

| Data category | Put the authority in | Request/reply shape | Reset and routing rules | Existing examples |
|---|---|---|---|---|
| **UI-only** | A component, or the `ui` slice in `src-webgui/src/store/koma.ts` | No bridge request. Use a normal store action for state shared by panels/tabs. | Reset with the component/store state. It must not be reconstructed from a Rust push. | `HelpTab`, `ui.tabs`, `ui.activeTabId`, sidebar open/collapse state |
| **Host-local** | The GUI host process, with direct filesystem, Git, or local-ledger access | Add `GuiReq`, dispatch to `HostCtl`, compute off the GUI event/fold loop, and emit a one-shot `PushEnvelope` | Must work with or without an attached daemon. Prefer an error/empty result over a request that leaves a spinner running. | `FileDiff`, `GitStatus`, `GitDiff`, `UsagePreview` |
| **Daemon/session-backed** | The attached session daemon and its authoritative session state | Add `GuiReq`, forward through `ctx.req` as a `ClientRequest`, and re-push the daemon reply | Requires an attached session unless a deliberate host fallback exists. Key replies by session or another request identity; invalidate on a session switch. | `Submit`/`Snapshot`, `GetEffortOptions`, `SetPrefs`, stream data |
| **Persisted/global** | Config or a global host store that survives sessions (for example `~/.koma/config.json`, the usage ledger, or the key vault) | Use the existing dual-route or host route: attached config requests use `forward_config_req`; pre-session mutations use `HostCtl::ConfigMutate`; local stores use their host worker. | Do not clear it merely because the foreground session changes. Push the authoritative saved value after mutations. | Connector providers/models/MCP/theme, SSH keys, all-scope usage |
| **Session-scoped** | The foreground session's daemon state or per-session on-disk state | Include or derive a session identity. For tab data also echo the resource key (path, SHA, etc.) and validate it in the reducer. | Reset/invalidate on the `Snapshot` session-change branch and on `detachSession`; never let session A's data render under session B. | `Snapshot.fileChanges`, chat/subagents/bash, session settings, a FileDiff's session baseline |

Before adding a field, decide whether it is authoritative, transient loading
state, or a display-only derivation. Store authoritative results in the relevant
Zustand slice/tab, and keep loading/error flags beside that result. For the full
UI-only menu/tab wiring (including singleton versus document tab identity), see
[`WEBGUI_SIDEBAR_TABS.md`](WEBGUI_SIDEBAR_TABS.md); this document covers the
bridge once a feature crosses the UI/host boundary.

## 1. JavaScript sends a request

The store's `req` action is the single request helper:

```ts
// src-webgui/src/store/koma.ts
req: (g) => {
  try {
    window.ipc?.postMessage(JSON.stringify({ t: 'req', ...g }))
  } catch {
    /* ipc unavailable */
  }
}
```

A component normally calls a store action instead of constructing the bridge
payload itself. For example:

```ts
refreshTasks: () => {
  get().req({ r: 'Tasks' })
},
```

This keeps the wire format and the feature's loading/state transitions in one
place. A tab can call `refreshTasks()` when it becomes active:

```tsx
const activeTabId = useKoma((s) => s.ui.activeTabId)
const refreshTasks = useKoma((s) => s.refreshTasks)

useEffect(() => {
  if (activeTabId === 'tasks') refreshTasks()
}, [activeTabId, refreshTasks])
```

The `Tasks` request above is illustrative. It does not exist until the same
request is added on the Rust side.

## 2. Rust receives and dispatches it

The wry IPC handler in `src-agent/src/app/runtime/gui/mod.rs` parses the JSON
message. `ClientMsg` and `GuiReq` are defined in
`src-agent/src/app/runtime/gui/proto.rs`:

```rust
#[serde(tag = "t")]
enum ClientMsg {
    Req(GuiReq),
    // window messages...
}
```

After deserialization, the GUI calls the dispatcher in
`src-agent/src/app/runtime/gui/dispatch.rs`:

```rust
ClientMsg::Req(req) => dispatch::handle_gui_req(req, &gui_ctx),
```

Add a new request variant to the Rust `GuiReq` enum:

```rust
#[serde(tag = "r")]
enum GuiReq {
    // existing variants...
    Tasks,
}
```

Then add a matching branch to `handle_gui_req`:

```rust
GuiReq::Tasks => {
    let _ = ctx.ctl.send(HostCtl::Tasks)
}
```

The exact control message depends on where the work belongs. Host-local work
can use a new `HostCtl` message. Work owned by the attached daemon can forward
a `ClientRequest` through `ctx.req`. Follow the existing `GitStatus`,
`FileDiff`, and `GetSettings` branches rather than creating a second dispatch
path.

### Attached versus detached state

The GUI can exist while no session is attached. Request routing must therefore
choose the appropriate owner:

- host-local features, such as Git status and file diffs, can work while
  detached and are dispatched through `HostCtl`;
- daemon/session features generally forward to the attached daemon;
- settings demonstrates `forward_or_host`, supporting both paths.

If a feature requires an attached session, return a deterministic error push
or an empty result. Do not leave the tab permanently loading.

## 3. Rust computes the result and emits a push

Push envelopes are serialized with the `k` discriminant in
`src-agent/src/app/runtime/client/push_proto.rs`:

```rust
#[serde(tag = "k")]
enum PushEnvelope {
    Tasks { items: Vec<TaskEntry>, error: Option<String> },
}
```

In practice, the result may be produced by a host worker, an attached daemon
reply interceptor, or a shared host helper. The important requirements are:

1. include enough identifying data for the client to match the reply to its
   current request/tab;
2. always emit a reply, including errors and empty results;
3. serialize field names to match the TypeScript contract;
4. avoid sending secrets or unnecessary large payloads.

The host emits the serialized envelope through the GUI push closure. The tao
event loop delivers it as a `UserEvent::Push`, and the webview invokes
`window.__komaClient.push(json)`.

For a keyed tab request, echo the key in the reply. Existing examples include:

```text
FileDiff     -> path
GitDiff      -> path + staged
CommitDiff   -> sha + path
UsagePreview -> scope + sessionId
```

This lets the reducer discard a stale reply that arrived after the user closed
the tab, changed a filter, or switched sessions.

## 4. JavaScript receives and reduces the push

The bridge callback is installed by `RootLayout` in
`src-webgui/src/routes/index.tsx`:

```tsx
window.__komaClient = {
  push: (j) => useKoma.getState().push(JSON.parse(j)),
}
```

Add the push variant to `PushEnvelope` in `store/koma.ts`:

```ts
export type TaskEntry = {
  id: string
  title: string
  status: 'pending' | 'done'
}

export type PushEnvelope =
  // existing variants...
  | {
      k: 'Tasks'
      items: TaskEntry[]
      error: string | null
    }
```

Add a store slice for the authoritative result and a reducer case:

```ts
type TasksSlice = {
  items: TaskEntry[]
  error: string | null
  loading: boolean
}
```

```ts
case 'Tasks':
  set(() => ({
    tasks: {
      items: env.items,
      error: env.error,
      loading: false,
    },
  }))
  break
```

The request action should set `loading: true` before calling `req`:

```ts
refreshTasks: () => {
  set((s) => ({ tasks: { ...s.tasks, loading: true, error: null } }))
  get().req({ r: 'Tasks' })
},
```

Use narrow selectors in the tab so unrelated pushes do not rerender it:

```tsx
const items = useKoma((s) => s.tasks.items)
const loading = useKoma((s) => s.tasks.loading)
const error = useKoma((s) => s.tasks.error)
```

For replies associated with a tab or filter, validate the echoed key inside the
reducer before replacing state. A reply for a closed tab should be ignored,
not resurrect the tab or overwrite current data.

## Existing implementations to copy

### `GitStatus`: host-local panel data

`GitStatus` is a good template for data that belongs to the host and is useful
without an attached session:

1. JavaScript calls `refreshGitStatus()` in `store/koma.ts`.
2. Rust deserializes `GuiReq::GitStatus` in `gui/proto.rs`.
3. `gui/dispatch.rs` sends `HostCtl::GitStatus`.
4. The host computes status in the client Git module.
5. Rust emits `PushEnvelope::GitStatus`.
6. The `GitStatus` reducer replaces the `git` store slice.
7. `GitPanel` and `UsageFooter` render the result.

Relevant Rust files include `dispatch_git.rs`, `git.rs`,
`push_proto_git.rs`, and the host relay modules.

### `FileDiff`: keyed tab data

`FileDiff` is a complete host-local vertical slice. See the detailed flow below.

### `FileDiff`: concrete end-to-end vertical slice

`FileDiff` is the best copyable example for a tab-specific host request. The
actual path is:

```text
ExplorePanel
  -> openDiffTab(path)
  -> req({ r: 'FileDiff', path })
  -> GuiReq::FileDiff
  -> HostCtl::FileDiff
  -> compute_file_diff()
  -> PushEnvelope::FileDiff
  -> window.__komaClient.push()
  -> case 'FileDiff' in useKoma.push()
  -> DiffTab
```

The implementation points are:

1. `src-webgui/src/components/sidebar/ExplorePanel.tsx` calls
   `openDiffTab(f.path)` when a changed file is clicked.
2. `src-webgui/src/store/koma.ts:2542` creates or focuses the stable
   `diff:<path>` tab, marks it `loading: true`, and sends
   `{ r: 'FileDiff', path }`.
3. `src-agent/src/app/runtime/gui/proto.rs` defines `GuiReq::FileDiff`.
4. `src-agent/src/app/runtime/gui/dispatch.rs` forwards it as
   `HostCtl::FileDiff`.
5. `src-agent/src/app/runtime/client/host.rs` receives the control message and
   starts the host-side worker.
6. `src-agent/src/app/runtime/client/diff.rs` runs `compute_file_diff`, reading
   the modified file and resolving its Git or virtual-Git baseline.
7. `src-agent/src/app/runtime/client/push_proto.rs` emits
   `PushEnvelope::FileDiff`, including `path`, `original`, `modified`,
   `error`, `binary`, and `origin`.
8. `src-webgui/src/store/koma.ts:2015` finds `diff:<path>`. If the tab was
   closed, it ignores the reply; otherwise it stores the `DiffPayload` and
   clears `loading`.
9. `src-webgui/src/components/DiffTab.tsx` renders the stored payload or its
   error/binary state.

`path` is echoed because the user can open several diff tabs and replies are
asynchronous. The same pattern applies to any tab whose request has a stable
resource key: include that key in the request, echo it in the push, and match it
in the reducer before updating state.

### `GetSettings`: attached and detached routing

`GetSettings` demonstrates a request that can be served by either the attached
daemon or the host fallback. Its dispatcher uses `forward_or_host`, and the
result arrives as `SettingsValues`. This is the pattern to follow when a tab
should remain useful on the start screen as well as inside a session.

## Feature lifecycle and stale replies

Treat every backend-backed feature as an asynchronous state machine. The store
owns the lifecycle, not the component that happens to render it:

- **Loading:** the store action marks the result as loading before it calls
  `req`. Clear the previous error, but do not pretend that the old result is the
  new response.
- **Success:** Rust emits a push containing the complete authoritative result;
  the reducer stores it and clears `loading`.
- **Empty:** an empty list, missing optional value, or "nothing found" result is
  still a successful reply. Emit it explicitly and render an empty state; do not
  use a missing push to mean empty.
- **Error:** Rust emits a structured error (or the feature's existing error
  field), and the reducer clears `loading` and stores a user-visible message.
  A request must not leave a permanent spinner while waiting for a timeout.

Replies can arrive after the UI has changed. A late reply for a tab that the user
closed must be ignored: the reducer must check that the tab still exists and that
the reply's resource key still matches it. It must not recreate the tab or write
to a replacement tab that happens to use the same component.

Requests can also complete out of order. Include a correlation key in the
request and echo it in the push. For tab data this normally includes the stable
resource key (such as `path`), and it may also need a request generation or
session identity. The reducer accepts a reply only when its correlation key is
still current; it must never assume that the last reply received is the newest
request sent.

Reactivating a tab is a refresh boundary. When a tab becomes active again, its
store action may issue a new request, mark the tab loading, and replace the
result only when the new correlation key matches. This prevents a cached or
late response from being treated as proof that the current resource is still
valid.

Session-scoped state must be invalidated on both a session switch and
`detachSession`. Include the session identity, or an equivalent generation, in
session-backed correlation where a reply could cross that boundary. A reply
from session A must not populate a tab or slice now showing session B, and a
reply from an old attached daemon must not repopulate state after detaching.
Host-local data can remain useful while detached, but it must follow its own
resource-key rules.

Global or persisted state is different: it is not session-scoped and must not be
cleared merely because the foreground session changes. Mutations should push the
saved authoritative global value after they complete. A detached daemon is not a
reason to leave a daemon-owned feature loading forever: route to an intentional
host fallback when one exists, or emit a deterministic empty/error reply and let
the UI render that state.

## Debugging the round trip

Start at the first stage that does not show the expected value and work toward
the push reducer. The following table maps common symptoms to the stage and
files to inspect:

| Symptom | Stage and files to inspect |
|---|---|
| Clicking the UI does not produce a request | **Store action:** the feature action in `src-webgui/src/store/koma.ts`; confirm it sets loading and calls `req`. |
| The request has the wrong tag or fields | **Web request contract:** `src-webgui/src/koma.d.ts` and the action's `{ r: ... }` payload. |
| Rust never receives or deserializes the request | **Webview bridge and Rust protocol:** `src-agent/src/app/runtime/gui/mod.rs` and `src-agent/src/app/runtime/gui/proto.rs` (`ClientMsg`/`GuiReq`). |
| Rust deserializes the request but nothing runs | **Dispatcher:** `src-agent/src/app/runtime/gui/dispatch.rs`; confirm the new `GuiReq` branch is present and sends the intended control message. |
| The request runs on the wrong owner, or fails only without a session | **Routing:** `dispatch.rs` and `dispatch_forward.rs`; check attached-daemon versus `HostCtl`/host-fallback behavior. |
| The host or daemon computes a result but no push appears | **Worker and relay:** the relevant `src-agent/src/app/runtime/client/` worker, `host.rs`/`push_loop.rs`, and the path that emits the GUI push. |
| A push is emitted but has the wrong tag or shape | **Push contract:** `src-agent/src/app/runtime/client/push_proto.rs`, `src-webgui/src/store/koma.ts`, and `src-webgui/src/koma.d.ts`. |
| Rust logs a push but the web UI never sees it | **Event and callback bridge:** `UserEvent::Push` in `src-agent/src/app/runtime/gui/mod.rs` and `window.__komaClient.push` in `src-webgui/src/routes/index.tsx`. |
| The push reaches JavaScript but the spinner never clears | **Reducer:** the matching `push()` case in `src-webgui/src/store/koma.ts`; verify success, empty, and error all set `loading: false`. |
| Data appears in the wrong tab or an old result wins | **Correlation and stale-reply guard:** the request key, echoed push key, and reducer check in `store/koma.ts`. |
| Data from another session reappears after switching or detaching | **Session reset:** the session-change branch and `detachSession()` in `src-webgui/src/store/koma.ts`; invalidate old keys and session-scoped slices. |

When debugging, log the request tag, correlation key, session identity (when
applicable), and push tag at each boundary. A successful computation is not a
successful round trip until the matching reducer accepts it.

## New-feature checklist

- [ ] Add the request type to the TypeScript request union in
      `src-webgui/src/koma.d.ts` and to the Rust `GuiReq` enum in
      `src-agent/src/app/runtime/gui/proto.rs`.
- [ ] Add the push type on both sides: the TypeScript `PushEnvelope` union in
      `src-webgui/src/store/koma.ts` and the Rust push serialization in
      `src-agent/src/app/runtime/client/push_proto.rs`.
- [ ] Add one store action that owns the request payload and lifecycle; call the
      shared `req` helper instead of posting IPC from a component.
- [ ] Add the `handle_gui_req` dispatcher branch in
      `src-agent/src/app/runtime/gui/dispatch.rs`.
- [ ] Decide who owns the data and implement the correct attached-daemon,
      host-local, fallback, or detached routing. Do not silently drop a request
      when no daemon is attached.
- [ ] Define and emit explicit push replies for success, valid empty results,
      and errors.
- [ ] Choose a correlation key, echo it in every reply, and validate it in the
      reducer before replacing state.
- [ ] Ignore replies for stale or closed tabs; do not resurrect tabs or let an
      older request overwrite a newer one.
- [ ] Reset or invalidate session-scoped state and correlation keys on session
      switches and `detachSession`; preserve global/persisted state across those
      transitions.
- [ ] Render distinct loading, success, empty, and error UI states, including
      the detached/no-session case when it is applicable.
- [ ] Keep authoritative data and lifecycle flags in the appropriate Zustand
      slice or tab, and use narrow selectors in the component.
- [ ] Run the relevant frontend build and Rust checks from the Verification
      section, then exercise the full round trip with tab close, reactivation,
      out-of-order replies, session switching, and detaching.

## Anti-patterns

Avoid these patterns when extending the bridge:

- **Direct IPC from components:** components should call a store action, not
  construct JSON or call `window.ipc` themselves.
- **TypeScript-only types:** adding a type to `koma.d.ts` does not add a Rust
  request, dispatcher branch, worker, or push. The request and push contracts
  must exist on both sides.
- **Component-only authoritative state:** do not keep backend-owned data only in
  a component. The store must own the result and its loading/error lifecycle so
  tabs can be switched and remounted safely.
- **Ordering assumptions:** asynchronous replies are not ordered. Do not accept
  the last reply received without checking its correlation key and session.
- **Timeout-only errors:** a timeout is not a reply. Emit deterministic error or
  empty pushes from Rust for every request; use timeouts only as an additional
  recovery mechanism.
- **Stale session reuse:** do not reuse session-scoped results, request keys, or
  pending state after a switch or detach unless they have been explicitly
  validated for the current session.
- **Duplicate IPC channels:** do not create a feature-specific bridge or bypass
  the existing `req`/`PushEnvelope` path. Add one typed request, one dispatcher
  route, and one reducer path to the established channel.

## Verification

For frontend-only changes:

```bash
cd src-webgui
npm run build
```

For Rust protocol or host changes:

```bash
cargo test
cargo build -p agent --features gui
```

Test the full round trip in the desktop GUI:

1. open the tab;
2. confirm the request leaves the webview;
3. confirm Rust emits a push for success, empty, and error cases;
4. switch tabs while the request is in flight;
5. close the tab while the request is in flight;
6. switch sessions and ensure old data does not reappear.

For the UI-only sidebar/tab pattern, see
[`WEBGUI_SIDEBAR_TABS.md`](WEBGUI_SIDEBAR_TABS.md). For the broader GUI
architecture, see [`ARCH_DESIGN_WEBGUI.md`](ARCH_DESIGN_WEBGUI.md).
