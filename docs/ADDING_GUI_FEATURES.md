# Web GUI: Adding a New Feature

This is the implementation recipe for extending the native web GUI. Use it when a
feature needs more than a small component change: a new sidebar capability, a new
tab, host data, daemon data, or a mutation that crosses the JS/Rust boundary.

The existing detailed guides are the source of truth for each layer:

- [`WEBGUI_SIDEBAR_TABS.md`](WEBGUI_SIDEBAR_TABS.md) — sidebar menu, panel, tab,
  tab bar, and tab-content wiring.
- [`WEBGUI_IPC.md`](WEBGUI_IPC.md) — request/push communication, Rust routing,
  ownership, lifecycle, and stale-reply handling.

This guide tells you which layers to touch and in what order.

## The complete path

A feature that opens from the sidebar and loads backend data normally follows
this path:

```text
ActivityBar
  -> RootLayout active SidebarView
  -> Sidebar panel
  -> Zustand open/refresh action
  -> ui.tabs + ui.activeTabId
  -> TabBar
  -> TabbedMain tab content
  -> store req({ r: '...' })
  -> Rust GuiReq and dispatcher
  -> HostCtl or attached daemon
  -> PushEnvelope
  -> useKoma.push reducer
  -> Zustand feature slice/tab state
  -> tab renders loading/success/empty/error
```

Do not implement only the visible React layer and call the feature complete. If
the feature crosses the bridge, every request and push must be represented on both
sides and must have a deterministic reply.

## 1. Decide the feature shape first

Before editing files, answer these questions:

1. Is the feature UI-only, or does it need Rust/daemon data?
2. Is the data host-local, daemon/session-backed, persisted/global, or
   session-scoped?
3. Does it need a sidebar panel, a main-column tab, or both?
4. Is the tab a singleton, or can users open multiple resource instances?
5. What is the stable identity of a tab or request?
6. What should happen with no attached session?
7. What happens when the user switches sessions, closes the tab, or activates it
   again while a request is in flight?

Use this decision table:

| Feature | Store/bridge shape |
|---|---|
| Static help or local display preference | Component state or the `ui` Zustand slice; no Rust request |
| Host filesystem, Git, or local-ledger data | Host-local `GuiReq`, `HostCtl` dispatch, one-shot push |
| Attached session or daemon state | `GuiReq` forwarded to the daemon, then re-pushed to the GUI |
| Config, key vault, or usage data shared across sessions | Global/persisted host route; do not reset on session switch |
| Data belonging to the foreground session | Session-aware request/reply and reset on switch/detach |

If the feature has a backend owner, keep the authoritative result in Zustand.
Keep loading and error state beside it. Component state is appropriate for a
local draft, form focus, or an interaction that no other component needs.

## 2. Map the files before editing

Most sidebar-and-tab features touch these frontend files:

| Purpose | File |
|---|---|
| ActivityBar entry | `src-webgui/src/components/ActivityBar.tsx` |
| Sidebar view type and panel routing | `src-webgui/src/components/Sidebar.tsx` |
| New panel | `src-webgui/src/components/panels/<Name>Panel.tsx` |
| Tab union, state, actions, and reducer | `src-webgui/src/store/koma.ts` |
| Tab strip | `src-webgui/src/components/TabBar.tsx` |
| Main tab routing | `src-webgui/src/routes/index.tsx` |
| New tab content | `src-webgui/src/components/<Name>Tab.tsx` |
| Request type | `src-webgui/src/koma.d.ts` |

A backend-backed feature usually also touches:

| Purpose | File or area |
|---|---|
| Rust request deserialization | `src-agent/src/app/runtime/gui/proto.rs` |
| Rust GUI dispatch | `src-agent/src/app/runtime/gui/dispatch.rs` |
| Host control and worker path | `src-agent/src/app/runtime/client/` |
| Rust push serialization | `src-agent/src/app/runtime/client/push_proto.rs` |
| Push reducer | `src-webgui/src/store/koma.ts` |

Read the closest existing implementation before changing it. For UI-only tab
plumbing, use Settings, Help, Graph, or a diff tab as the template. For a
host-local request, use `FileDiff` or `GitStatus`. For attached/detached routing,
use `GetSettings`.

## 3. Build the UI-only skeleton

Complete the UI path before adding backend behavior. This makes tab identity and
rendering independently testable.

### Add the sidebar view

In `Sidebar.tsx`:

- add a literal to `SidebarView`;
- add its title to the titles map;
- import the new panel;
- render the panel in the existing view switch.

`RootLayout` already handles selecting a view and opening/collapsing the sidebar.
A normal new menu does not require a new RootLayout state path.

### Add the ActivityBar item

In `ActivityBar.tsx`, add a `lucide-react` icon and an item whose `view` exactly
matches the new `SidebarView` literal. The shared type should catch mismatches.

### Add the panel

Create `src-webgui/src/components/panels/<Name>Panel.tsx`. The panel should
call a store action to open or focus the main tab:

```tsx
const openFeatureTab = useKoma((s) => s.openFeatureTab)

<button onClick={openFeatureTab}>Open feature</button>
```

Do not use refs or prop drilling to reach the main-column tab. The store action is
the boundary between the sidebar and the editor area.

### Add tab identity and actions

Add the new tab variant to the `Tab` union in `src-webgui/src/store/koma.ts`.
Use a fixed ID for a singleton:

```ts
| { id: 'feature'; kind: 'feature' }
```

Use a resource-derived ID when multiple instances are valid:

```text
document:<resource-key>
```

The ID must be stable enough to deduplicate repeated opens. Add the action to
`KomaState` and implement it as open-or-focus:

```ts
openFeatureTab: () => {
  set((s) => {
    const exists = s.ui.tabs.some((t) => t.id === 'feature')
    const tabs: Tab[] = exists
      ? s.ui.tabs
      : [...s.ui.tabs, { id: 'feature', kind: 'feature' }]
    return { ui: { ...s.ui, tabs, activeTabId: 'feature' } }
  })
},
```

Repeated clicks must focus the existing singleton rather than append duplicates.
For a document tab, deduplicate by its stable resource ID instead.

### Render the tab

Create `src-webgui/src/components/<Name>Tab.tsx`, then add a matching branch in
`TabbedMain` in `src-webgui/src/routes/index.tsx`. Follow the existing pattern:

- use `key={t.id}`;
- place content in an absolute full-size layer;
- hide inactive tabs with the existing `hidden` class;
- keep inactive tabs mounted unless the feature has a specific reason not to;
- use `min-h-0 flex-1 overflow-y-auto` for scrollable content.

Add the matching tab-strip branch in `TabBar.tsx`. It should activate on click,
close through the shared `closeTab` action, stop propagation from the close button,
and support keyboard activation when it uses `role="button"`.

For the exact snippets and styling conventions, stop here and follow
[`WEBGUI_SIDEBAR_TABS.md`](WEBGUI_SIDEBAR_TABS.md).

## 4. Add backend communication only when needed

A static tab ends after the UI skeleton. A backend-backed tab needs the complete
vertical slice below.

### Define the request contract

Add the request to the TypeScript `GuiReq` union in
`src-webgui/src/koma.d.ts` and to the Rust `GuiReq` enum in
`src-agent/src/app/runtime/gui/proto.rs`.

Keep the payload small and explicit. Include the resource identity needed to
match the reply:

```text
FileDiff       -> path
GitDiff        -> path + staged
CommitDiff     -> sha + path
UsagePreview   -> scope + sessionId
```

Do not invent a second JavaScript-to-Rust channel. Store actions must call the
shared `req` helper, which adds the `{ t: 'req' }` envelope and sends it through
`window.ipc`.

### Choose and implement Rust routing

Add a branch to `handle_gui_req` in `src-agent/src/app/runtime/gui/dispatch.rs`.
Choose the owner deliberately:

- host-local work goes through `HostCtl` and a host worker;
- attached daemon work forwards a `ClientRequest` and re-pushes the reply;
- features useful both attached and detached can follow the existing
  `forward_or_host` pattern;
- persisted/global mutations must push the saved authoritative value afterward.

If no session is attached, do not silently drop the request. Use a host fallback,
or emit a deterministic empty/error reply so the UI can clear its loading state.

### Define and emit the push

Add the Rust push variant in
`src-agent/src/app/runtime/client/push_proto.rs` and the matching TypeScript
variant in the `PushEnvelope` union in `src-webgui/src/store/koma.ts`.

Every request must produce a reply for:

- successful data;
- valid empty data;
- errors and unavailable/no-session cases.

Echo the resource/filter/session identity in the reply. A reply without enough
identity cannot be safely reduced when users switch tabs or sessions quickly.

### Add Zustand state and reducer behavior

Add the authoritative result and lifecycle flags to the appropriate store slice
or tab. A typical shape is:

```ts
type FeatureSlice = {
  items: FeatureEntry[]
  loading: boolean
  error: string | null
}
```

The store action sets loading before sending:

```ts
refreshFeature: () => {
  set((s) => ({ feature: { ...s.feature, loading: true, error: null } }))
  get().req({ r: 'FeatureList' })
},
```

The reducer must replace authoritative data, clear loading on every reply, and
reject stale replies. For a tab, first verify that the tab still exists and that
the echoed resource key maps to the current tab. For filters or selections,
compare the echoed filter/SHA/path with the current store value. For
session-scoped data, also verify the current session or invalidate the state on
switch/detach.

A late reply must never recreate a closed tab or overwrite a newer request.

For the full Rust-side flow and lifecycle rules, follow
[`WEBGUI_IPC.md`](WEBGUI_IPC.md).

## 5. Define the feature lifecycle

Before polishing the UI, write down the expected transitions:

```text
open or activate
  -> loading
  -> success with data
  -> success with empty data
  -> error
  -> closed while loading: ignore late reply
```

Implement these cases explicitly:

- show loading while the request is in flight;
- render empty as a valid result, not as a missing response;
- show a user-readable error and clear loading on failure;
- keep old content during refresh when that avoids unnecessary flicker;
- re-request on activation only when the feature needs freshness;
- reset session-owned state on a genuine session switch and `detachSession`;
- preserve global/persisted data across session changes;
- handle an attached daemon disappearing without leaving a spinner forever.

When multiple requests can race, use an echoed correlation key and, if needed, a
monotonic request generation. Never rely on response order.

## 6. Handle mutations differently from snapshots

For a mutation, do not optimistically rewrite authoritative lists unless the
existing feature has a clear rollback strategy. Prefer this sequence:

```text
user action
  -> store action sends mutation request
  -> Rust performs mutation
  -> one-shot operation result (success/error)
  -> fresh authoritative list/status push
  -> reducer replaces the list/status
```

Keep a failed form draft when it contains user input. Surface operation errors as
the feature's inline error or the existing toast mechanism, matching neighboring
features.

For secrets, tokens, private keys, prompts, and other sensitive values, send only
what the host needs and push only what the GUI must display. Never add credentials
to diagnostic logging.

## 7. Verify the whole feature

Run checks appropriate to the layers changed:

```bash
# Frontend changes
cd src-webgui
npm run build

# Rust protocol/host changes, from the repository root
cargo test
cargo build -p agent --features gui
```

Then exercise the actual GUI path:

- open the sidebar menu and confirm the panel appears;
- click the panel action twice and confirm singleton deduplication;
- activate the tab and confirm the request payload;
- verify success, empty, and error replies;
- switch away and back while data is loading;
- close the tab while a request is in flight;
- issue two requests with different filters/resources and deliver replies out of
  order;
- switch sessions while a reply is in flight;
- detach the session and confirm old session data does not reappear;
- confirm global/persisted data remains available across session changes;
- verify keyboard activation and tab closing behavior.

## Final implementation checklist

- [ ] Ownership and scope are documented before code is written.
- [ ] Singleton versus resource-derived tab identity is explicit.
- [ ] ActivityBar, Sidebar, panel, store, TabBar, and TabbedMain are wired.
- [ ] Backend requests exist in both TypeScript and Rust when needed.
- [ ] Rust dispatch uses the correct host or daemon route.
- [ ] Push envelopes exist in both languages and use matching field names.
- [ ] Loading, success, empty, error, and unavailable states are implemented.
- [ ] Replies echo and validate resource/filter/session identity.
- [ ] Closed tabs and stale replies are ignored.
- [ ] Session-scoped state resets on switch and detach.
- [ ] Mutations are followed by an authoritative refresh.
- [ ] Sensitive data is not unnecessarily pushed or logged.
- [ ] Frontend, Rust, and GUI feature-build checks pass.
