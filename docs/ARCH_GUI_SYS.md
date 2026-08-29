# ARCH_GUI_SYS.md — Koma WebGUI Frontend Architecture

This document covers the React/TypeScript frontend inside `src-webgui/`: every
component, the single Zustand store, the IPC bridge to the Rust host, the
layout system, sidebar, tabs, accordions, streaming, approval, onboarding, theme
engine, and every data-flow path end-to-end.

---

## Table of Contents

1. [Stack and File Map](#1-stack-and-file-map)
2. [Boot Sequence](#2-boot-sequence)
3. [IPC Bridge: Rust <-> JS](#3-ipc-bridge-rust---js)
4. [Wire Protocol](#4-wire-protocol)
5. [Zustand Store](#5-zustand-store)
6. [Layout Architecture](#6-layout-architecture)
7. [Titlebar](#7-titlebar)
8. [Activity Bar and Sidebar](#8-activity-bar-and-sidebar)
9. [Tab System](#9-tab-system)
10. [Chat View](#10-chat-view)
11. [Composer](#11-composer)
12. [Streaming and Token Rendering](#12-streaming-and-token-rendering)
13. [Tool Call Display](#13-tool-call-display)
14. [Approval and Plan Decision](#14-approval-and-plan-decision)
15. [Accordion System](#15-accordion-system)
16. [Explore Panel](#16-explore-panel)
17. [Connector Panel](#17-connector-panel)
18. [MCP Panel](#18-mcp-panel)
19. [OmniSearch Palette](#19-omnisearch-palette)
20. [Resume Palette and Session Switching](#20-resume-palette-and-session-switching)
21. [Diff Tabs and Monaco Editor](#21-diff-tabs-and-monaco-editor)
22. [File Attachment Flow](#22-file-attachment-flow)
23. [Onboarding](#23-onboarding)
24. [Start Screen](#24-start-screen)
25. [Toasts](#25-toasts)
26. [Theme System](#26-theme-system)
27. [Animations](#27-animations)
28. [Keyboard Shortcuts](#28-keyboard-shortcuts)
29. [Component Inventory](#29-component-inventory)
30. [Key Constants and Limits](#30-key-constants-and-limits)

---

## 1. Stack and File Map

**Runtime:** React 19, TypeScript 5.7, Vite 6, Tailwind CSS v4
**State:** Zustand 5 (single store, ~885 lines)
**Routing:** TanStack Router 1.87 (hash history, two routes: `/` and `/settings`)
**Animations:** Framer Motion 12
**Markdown:** Streamdown 2.5 (Vercel's streaming-safe renderer)
**Code highlighting:** Shiki via `@streamdown/code` 1.1 (JS regex engine, no WASM)
**Editor:** Monaco Editor 0.55 (lazy-loaded, diff tabs only)
**Lottie:** `lottie-react` 2.4 (cat mascot animations)
**Icons:** `lucide-react` 1.23

### File tree (48 source files)

```
src-webgui/
  index.html                          # Minimal HTML shell, <div id="root">
  package.json                        # Dependencies
  tsconfig.json                       # ES2020, bundler resolution, noEmit
  vite.config.ts                      # 3 plugins: react, tailwindcss, lottie
  vite-plugin-lottie.ts               # Build-time .lottie extractor
  public/
    fonts/                            # JetBrains Mono 400/500/700 woff2
    lottie/                           # 4 cat .lottie animation archives
  src/
    main.tsx                          # ReactDOM entry
    router.tsx                        # Hash router, route tree
    routes/index.tsx                  # RootLayout + IndexPage + TabbedMain
    styles.css                        # Tailwind v4 theme, @font-face, global chrome
    koma.d.ts                         # Window/global bridge types + GuiReq union
    vite-env.d.ts                     # Virtual module decl for lottie
    store/koma.ts                     # Single Zustand store (885 lines)
    types/config.ts                   # McpServer, Provider, Model, Role, Scope
    lib/toolSignature.ts              # Tool call display formatting
    components/
      AccordionSection.tsx            # Reusable collapsible section
      ActivityBar.tsx                 # Icon strip (left rail)
      ApprovalOverlay.tsx             # Risky tool approval modal
      CatMascot.tsx                   # Persistent cat mascot (lazy lottie)
      CatMascotLottie.tsx             # Lottie player leaf
      ChatView.tsx                    # Full chat view (messages, streaming, tools)
      Composer.tsx                    # Message input + attach + pickers
      DiffTab.tsx                     # Monaco DiffEditor (lazy)
      MessageBody.tsx                 # Streamdown markdown renderer
      ModeSelector.tsx                # Auto/Plan/Normal dropdown
      ModelPicker.tsx                 # Session model quick-picker
      OmniSearchPalette.tsx           # Workspace file search overlay
      Onboarding.tsx                  # First-run setup wizard
      RenameOverlay.tsx               # Session rename overlay
      ResizeHandles.tsx               # Custom window resize handles
      ResumePalette.tsx               # Session switcher overlay
      Sidebar.tsx                     # Sidebar shell (3 views)
      StartScreen.tsx                 # Pre-session landing page
      SwitchingOverlay.tsx            # Session swap loading screen
      TabBar.tsx                      # Tab strip
      Titlebar.tsx                    # Custom frameless titlebar
      ToastContainer.tsx              # Transient toast notifications
      UsageFooter.tsx                 # Statusline
      komaShiki.ts                    # Trimmed Shiki (16 langs, JS engine)
      panels/
        ConnectorPanel.tsx            # Provider/Model/OAuth management
        ExplorePanel.tsx              # Plan/Files/Bash/Agents sidebar
        McpPanel.tsx                  # MCP server management
        form.tsx                      # Reusable form primitives
        helpers.tsx                   # Row, Empty, ScopePill, DetailHeader
        connector/
          ConnectorListView.tsx       # 3-accordion list (Providers/OAuth/Models)
          ModelForm.tsx               # Model create/edit with route picker
          OAuthConnect.tsx            # OAuth connect flow (Connector)          ProviderForm.tsx            # Provider create/edit with presets
        mcp/
          McpEditView.tsx             # MCP server form
          McpListView.tsx             # MCP server list with toggle/delete
```

---

## 2. Boot Sequence

1. **`src/main.tsx`**: `ReactDOM.createRoot` on `#root`, renders `<StrictMode>` wrapping `<RouterProvider>`.

2. **`src/router.tsx`**: TanStack Router with hash history; primary route is
   `RootLayout` / session chrome. Settings open as a **main-area tab**
   (`openSettingsTab`), not a dead `/settings` stub route.

3. **`src/routes/index.tsx` — `RootLayout`**:
   - Resolves `window.__komaOS` once (platform detection, injected by Rust host before boot).
   - Sets up the bridge: `window.__komaClient = { push: (j) => useKoma.getState().push(JSON.parse(j)) }`.
   - Fires `useKoma.getState().req({ r: 'Ready' })` to announce readiness — the host responds with its first push (Hub or Snapshot).
   - Renders the frameless window chrome.

4. **`src/routes/index.tsx` — `IndexPage`** (three-way gate):
   - `useNeedsOnboarding()` returns true → `<Onboarding />` (no other chrome).
   - `sessionId === null` → `<StartScreen />`.
   - Otherwise → `<TabbedMain />` (ChatView + TabBar + diff tabs).

5. **`useNeedsOnboarding()`** returns true when `config.loaded && sessionId === null && (firstRun ?? !(providers.length > 0 && models.some(m => m.roles.includes('main'))))`. This prevents flashing the start screen before the first Config push.

---

## 3. IPC Bridge: Rust <-> JS

The frontend communicates with the Rust host through two channels:

### Rust → JS: `window.__komaClient.push(json)`

The Rust host calls `evaluate_script("window.__komaClient.push(\"...\")")` on the webview. The bridge is wired in `RootLayout`'s `useEffect` (routes/index.tsx:67-75):

```ts
window.__komaClient = {
  push: (j) => useKoma.getState().push(JSON.parse(j)),
}
```

The `json` argument is a **complete JSON object** — not a string to parse, but an object that the host serializes into the script call. The push reducer (`store/koma.ts`) dispatches on `env.k`.

### JS → Rust: `window.ipc.postMessage(msg)`

The `req()` action in the store serializes `{ t: 'req', ...guiReq }` and posts it:

```ts
req: (g: GuiReq) => window.ipc?.postMessage(JSON.stringify({ t: 'req', ...g }))
```

Also used directly by:
- **Titlebar**: `{ t: 'win', a: 'drag'|'min'|'max'|'close' }` — window controls.
- **ResizeHandles**: `{ t: 'winresize', dir }` — edge/corner resize.

### Global types (`src/koma.d.ts`)

```ts
interface KomaClient { push(json: string): void }
interface Window {
  __komaOS?: string
  __komaClient?: KomaClient
  ipc?: { postMessage(msg: string): void }
}
```

---

## 4. Wire Protocol

### Push Envelope (Rust → JS)

Defined as `PushEnvelope` union in `store/koma.ts:187-304`, discriminated on `k`:

| Variant | Key Fields | Purpose |
|---------|-----------|---------|
| `Snapshot` | `session`, `state`, `messages`, `title`, `palette`, `subagents`, `bash`, `fileChanges?`, `planTodos?`, `attachments`, `mode?`, `pendingSteer?`, `awaitingApproval?`, `approvalReason?`, `pendingCall?` | Full state replacement — the authoritative session snapshot |
| `Switching` | `to: string` | Swap-START signal (target session id) |
| `StreamMsg` | `session`, `text` | Streaming token — full accumulated text, not a delta |
| `Reasoning` | `session`, `text` | Streaming reasoning — full accumulated text |
| `Status` | `session`, `working`, `toast?`, `toastKind?`, `tokensIn?`, `tokensCached?`, `tokensOut?`, `cost?`, `mode?` | Turn state + usage counters + toast |
| `Hub` | `state`, `cooking`, `history` | Session list for swapper |
| `SearchResults` | `query`, `items` | OmniSearch file results |
| `Config` | `mcp`, `providers`, `models`, `palette?`, `firstRun?`, `theme?`, `themes?` | Full config replacement |
| `ModelList` | `provider`, `models` | Live model-id catalogue for a provider |
| `RouteList` | `provider`, `modelId`, `routes` | Live OpenRouter endpoint list for a model |
| `FileDiff` | `path`, `original`, `modified`, `error`, `binary` | Diff contents for Monaco editor tab |

**Critical design decision: There are NO incremental deltas.** Every `StreamMsg`/`Reasoning` push carries the **full accumulated string**. Every `Snapshot` replaces the **entire messages array**. The client is purely reactive — it never accumulates or appends.

### GuiReq (JS → Rust)

Defined in `src/koma.d.ts:4-126` as a discriminated union on `r` (33 variants):

**Session control:** `Ready`, `Submit{text}`, `SelectSession{id}`, `NewSession`, `RefreshHub`, `CancelSwitch`, `Rename{name}`, `Interrupt`, `RewindTo{index}`, `Compact`, `SetMode{mode}`, `SetSessionMain{modelUuid?}`

**Attachments:** `AttachFile{name, bytesB64, mime?}`, `AttachPath{path}`, `FileSearch{query}`, `RemoveAttachment{markerN}`

**Sub-agent/bash:** `KillSubagent{id}`, `BackgroundSubagent{id}`, `BackgroundAll`, `KillBash{id}`  
(`BackgroundAll` = composer Ctrl+B: all blocking sub-agents **and** still-blocking FG bash jobs.)

**Config CRUD:** `SetProvider`, `DeleteProvider`, `SetModel`, `DeleteModel`, `SetMcpServer`, `DeleteMcpServer`, `EnableMcpServer`

**Discovery:** `ListModels{provider}`, `ListRoutes{provider, modelId}`

**Approval:** `ApproveTool{approve}`, `PlanDecision{decision: 'approve'|'compact'|'deny'}`

**Onboarding:** `SetTheme{name}`, `SetupKomaFree`

**Diff:** `FileDiff{path}`

---

## 5. Zustand Store

Single store at `src/store/koma.ts` (885 lines), created via `create<KomaState>()`.

### Slices

**`SessionSlice`** — per-session state:
- `id`, `state`, `messages` (ChatMessage[]), `title`, `working`
- `stream` (live accumulated response text), `reasoning`
- `subagents` (SubAgentEntry[]), `bash` (BashJobEntry[]), `fileChanges`, `planTodos`
- `attachments` (AttachmentEntry[]), `searchResults`
- `mode` (`auto`/`plan`/`normal`/`yolo`)
- `pendingSteer` (queued mid-turn submits)
- `awaitingApproval`, `approvalReason`, `pendingCall`
- `tokensIn`, `tokensCached`, `tokensOut`, `cost`

**`HubSlice`** — swapper state:
- `state`, `cooking` (HubCookingEntry[]), `history` (HubHistoryEntry[])

**`ConfigSlice`** — global configuration:
- `mcp[]`, `providers[]`, `models[]`
- `loaded` (first Config pushed), `firstRun?`, `theme`, `themes[]`

**`UiSlice`** — local-only UI state:
- `omnisearchOpen`, `composerInsert`, `composerRefill`
- `pendingRewindIndex`, `scrollTick`
- `switchingTo`, `toast`, `toastSeq`
- `tabs` (Tab[]), `activeTabId`
- `focusPlanTick`

**Top-level fields:** `palette` (PaletteColors), `modelList`, `routeList`

### Key Actions

**`push(env: PushEnvelope)`** — the Rust→JS reducer. Giant `switch` on `env.k`:

- `Snapshot`: Wholesale replaces session fields. If `session.id` differs (session switch), clears `stream`/`reasoning`, resets tabs to chat-only. Calls `applyPaletteVars(env.palette)`.
- `StreamMsg`: `session.stream = env.text` (full replacement).
- `Reasoning`: `session.reasoning = env.text` (full replacement).
- `Status`: Updates `working`, usage counters, `mode`. Raises a new toast (deduped by text) when `toast` changes; bumps `toastSeq`.
- `Hub`: Replaces hub slice, clears `switchingTo`.
- `Config`: Replaces config slice wholesale. Applies palette CSS vars. Sets `config.loaded = true`.
- `FileDiff`: Finds tab by `diff:${path}`, fills `diff` payload, sets `loading: false`.

**`req(g: GuiReq)`** — JS→Rust. `window.ipc?.postMessage(JSON.stringify({ t: 'req', ...g }))`.

**Helper actions:**
- `openOmniSearch` / `closeOmniSearch` — toggle omnisearch overlay
- `insertToComposer` / `consumeComposerInsert` — one-shot path insertion from omnisearch
- `refillComposer` / `consumeComposerRefill` — one-shot text replacement for rewind
- `stageRewind` / `clearRewind` — pending rewind index for edit-pencil flow
- `requestScrollBottom` — bump `scrollTick` to force scroll anchor
- `startSwitching` / `cancelSwitching` — optimistic session swap loader
- `dismissToast` — clear toast by id (only if id matches current)
- `openDiffTab` / `closeTab` / `activateTab` — diff tab lifecycle
- `focusPlanSection` — bump `focusPlanTick` to open sidebar + plan accordion

### Wire Types (all in `store/koma.ts`)

- `ToolCallView` — `{ id, name, args, signature?, output: string|null, status: 'pending'|'done' }`
- `ChatMessage` — `{ role, kind?, content, reasoning, toolCalls?, attachments? }`
- `PaletteColors` — `{ bg, fg, accent, dim, panel }` (hex strings)
- `HubCookingEntry` — `{ kind, id?, name, working?, foreground?, dirLabel?, currentDir? }`
- `HubHistoryEntry` — `{ id, name, lastActive, dirLabel, currentDir }`
- `SubAgentEntry` — `{ id?, name, status, summary }`
- `BashJobEntry` — `{ id, cmd, status }`
- `FileChangeEntry` — `{ path, status: 'added'|'modified'|'deleted' }`
- `PlanTodoEntry` — `{ content, status: 'pending'|'in_progress'|'completed'|'cancelled' }`
- `AttachmentEntry` — `{ markerN, name, kind: 'image'|'file' }`
- `SearchResultEntry` — `{ path, label }`
- `PendingCall` — `{ name, args, signature? }`
- `ToastEntry` — `{ id, text, kind: 'error'|'info' }`
- `DiffPayload` — `{ original, modified, error: string|null, binary }`
- `Tab` — `{ id:'chat', kind:'chat' } | { id, kind:'diff', path, title, diff?, loading }`

### Data Types (`src/types/config.ts`)

- `Transport` — `'stdio' | 'http'`
- `McpServer` — `{ id, name, enabled, transport, command, args, env, url }`
- `Provider` — `{ id, name, endpoint, hasKey, isKomaFree? }`
- `OAuthProv` — `'OpenAI' | 'Kilo Code' | 'Anthropic'`
- `Scope` — `'global' | 'local'`
- `Role` — `'main' | 'awareness' | 'safeguard' | 'compactor' | 'planner'`
- `Model` — `{ id, name, modelId, provider, route, roles, scope, free? }`
- `ModelListEntry` — `string`
- `RouteEntry` — `{ name?, providerName, pricePrompt?, priceCompletion?, uptimeLast30m? }`

---

## 6. Layout Architecture

The page is a frameless window rendered inside `#app` (a transparent-background container with rounded corners).

```
+------------------------------------------------------------------+
|  #app (absolute, inset:0, rounded, bg: --koma-bg)                |
|  +--------------------------------------------------------------+|
|  | Titlebar (32px, z-10)                                        ||
|  | [traffic lights]  [change session pill] [rename pill] [cmd]  ||
|  +--------------------------------------------------------------+|
|  | ActivityBar | Sidebar (resizable) | main                     ||
|  | (48px)      | (150-500px)         | (<Outlet />)            ||
|  |  [Files]    |  [view header]      |  TabBar                 ||
|  |  [Blocks]   |  [panel content]    |  ChatView / DiffTab     ||
|  |  [Plug]     |                     |  ...                     ||
|  |  [Settings] |                     |  Composer                ||
|  |             |                     |  UsageFooter             ||
|  +--------------------------------------------------------------+|
|  [ResumePalette overlay]                                         |
|  [RenameOverlay overlay]                                         |
|  [OmniSearchPalette overlay]                                     |
|  [SwitchingOverlay — full-screen loader]                         |
|  [ToastContainer — top-right]                                    |
|  [ResizeHandles — 8 edge/corner handles]                         |
+------------------------------------------------------------------+
```

**Key layout details:**
- The `ActivityBar` + `Sidebar` + `main` are in a `flex` row.
- The sidebar is resizable via a 5px drag grip (clamped 150–500px). A `startResize` handler tracks mouse on window for smooth drag.
- The `main` column uses `flex-1 min-w-0` to fill remaining width.
- The `.term-shell` class sets `max-width: 1024px` on the chat column, centered.
- Diff tabs use the full width of `main`.
- `#app` has `border-radius: 10px` (16px on macOS). The Rust host makes the window transparent, so `#app` is the visible canvas.

### Three-way content gate (`IndexPage`)

1. **Onboarding** → renders only `<Titlebar>` + `<Onboarding>` (no sidebar, no activity bar).
2. **No session** (`sessionId === null`) → `<StartScreen />`.
3. **Session attached** → `<TabbedMain />`.

---

## 7. Titlebar

**`Titlebar.tsx`** — Custom frameless titlebar, 32px tall, `z-index: 10`.

### Layout

```
[traffic lights (macOS)]  [change session pill] [rename pill]        [min] [max] [close]
```

### Features

- **Drag region**: The entire titlebar is draggable (`window.ipc.postMessage({ t: 'win', a: 'drag' })` on mousedown).
- **Window controls**: Minimize/maximize/close buttons send `{ t: 'win', a: 'min'|'max'|'close' }`.
- **Platform-aware**: macOS gets traffic-light circles on the left (red/yellow/green) with hover-reveal glyphs. Linux/Windows get text-based buttons on the right.
- **"Change session" pill**: Triggers `onSearch` → opens `ResumePalette` overlay. Uses Framer Motion `layoutId="cmd-search"` for morph animation.
- **Rename pill**: Triggers `onRename` → opens `RenameOverlay`. Uses `layoutId="cmd-rename"` for morph animation.
- **`overlayOpen`**: When true, hides the cmd bar pills (used during onboarding and when an overlay is already open).

---

## 8. Activity Bar and Sidebar

### ActivityBar (`ActivityBar.tsx`)

A 48px-wide vertical icon strip on the left edge, VSCode-style:

| Icon | View | Notes |
|------|------|---------|
| Files | explore | Plan / files / bash / agents |
| Code2 | coding | File tree + Monaco coding tabs + host LSP |
| GitBranch | git | Source control |
| VectorSquare | mcp | MCP servers |
| Brain | connector | Providers / models / OAuth |
| Network | importGraph | Import graph |
| Bot | agents | Agent definitions |
| ChartColumn | usage | Usage |
| Blocks | store | Extension marketplace |
| Server | remote | Remote SSH hosts |
| (footer) | Settings / Help / Tutorial | Not SidebarView — open settings/help tabs or tutorial |

**Navigation:** clicking the active view toggles the sidebar; a different view
switches and opens the sidebar. Extension panel icons can share the bar’s
order/hidden/overflow machinery.

### Sidebar (`Sidebar.tsx`)

Resizable panel (≈150–500px) with header title. Renders the panel for
`SidebarView`:

`explore` · `coding` · `git` · `mcp` · `connector` · `importGraph` · `agents` · `usage` · `store` · `remote`

---

## 9. Tab System

### TabBar (`TabBar.tsx`)

VSCode-style tab strip at the top of the main content area.

**Structure:**
- `tabs[0]` is always the permanent, uncloseable **chat tab** (MessageSquare icon).
- **Diff tabs** appear when the user clicks a file-changed row in the Explore panel. They have a FileDiff icon, basename as title, and a close button.
- The TabBar is **hidden entirely** when only the chat tab exists (zero chrome cost).
- Active tab gets a 2px top accent line and raised background.
- When two diff tabs share a basename, a dim parent-dir suffix disambiguates (VSCode pattern).
- Tab close falls back to the left neighbour (tabs[0] is always there).

### Tab Types

```ts
type Tab =
  | { id: 'chat'; kind: 'chat' }                          // permanent
  | { id: `diff:${path}`; kind: 'diff'; path; title;      // opened from Explore
      diff?: DiffPayload; loading: boolean }
```

### Tab Lifecycle

1. **Open**: `openDiffTab(path)` — finds or creates a tab, marks `loading: true`, fires `FileDiff` req, activates it.
2. **Activate**: `activateTab(id)` — switches the visible content. For diff tabs, re-requests `FileDiff` for freshness while keeping the stale diff on screen (no flash).
3. **Close**: `closeTab(id)` — removes the tab; if it was active, activates the left neighbour. Chat tab cannot close.
4. **Session switch**: Resets tabs to `[makeChatTab()]`.

### TabbedMain (`routes/index.tsx:192-215`)

```tsx
<div className="flex h-full w-full min-w-0 flex-col">
  <TabBar />
  <div className="relative min-h-0 flex-1">
    {/* Chat is ALWAYS mounted (hidden via CSS when diff tab active) */}
    <div className={chatActive ? '' : 'hidden'}>
      <ChatView />
    </div>
    {/* Diff tabs mount on open, stay mounted while open, unmount on close */}
    {tabs.map(t =>
      t.kind === 'diff' ? (
        <div key={t.id} className={activeTabId === t.id ? '' : 'hidden'}>
          <Suspense fallback={<DiffFallback />}>
            <DiffTab tab={t} />
          </Suspense>
        </div>
      ) : null,
    )}
  </div>
</div>
```

The chat is hidden via CSS `hidden` class when a diff tab is active — it is **never unmounted**, preserving scroll position, streaming state, and DOM.

---

## 10. Chat View

**`ChatView.tsx`** (473 lines) — the full chat view. Layout:

```
+------------------------------------------------------------------+
| <div className="term-shell flex flex-col">                        |
|   <div ref={scrollRef} className="flex-1 overflow-y-auto">       |
|     {messages.map(m => <Message />)}       // committed messages  |
|     {showLive && <AssistantMessage />}      // live streaming     |
|   </div>                                                         |
|   <ApprovalOverlay />                     // tool approval card   |
|   <Composer />                            // input area           |
|   <UsageFooter />                         // statusline           |
| </div>                                                           |
+------------------------------------------------------------------+
```

### Message Rendering

The `Message` function dispatches by role and kind:

**User messages:**
- `kind === 'shell'` → `ShellMessage`: `$ <cmd>` header in accent, output in dim.
- `kind === 'bashNudge'` → `BashNudgeMessage`: Single dim line with success Check icon.
- Default → `UserMessage`: Full-width accent band (3px left rail in accent + accent text on panel bg). Edit pencil appears on hover (absolute positioned, `group-hover` reveal). Attachments shown via `AttachmentCard`.

**Assistant messages** (memoized, line 345):
- Circle bullet (9px filled accent circle).
- `ReasoningBlock` — collapsible thinking channel (Brain icon). Opens by default while streaming, auto-collapses when turn completes. Dim + italic + left border.
- Markdown body via `MessageBody`.
- Tool calls via `ToolCallRow` (see section 13).

### Scroll Management

- **Scroll anchor**: `stickRef` + `useLayoutEffect` keeps the view pinned to the bottom during streaming.
- **Scroll-on-send**: `scrollTick` (bumped by `requestScrollBottom`) forces re-engagement.
- **Live bubble keying**: The live streaming `<AssistantMessage>` is keyed at `messages.length`, so when the committed message arrives in the next Snapshot, React reuses the same DOM node — no flash.

---

## 11. Composer

**`Composer.tsx`** (375 lines) — pinned at the bottom of the chat view.

### Layout

```
+---------------------------------------------------------------+
| [pending steer queue indicator] (max 5 items)                 |
| +-----------------------------------------------------------+ |
| | [CatMascot (absolute top-right)]                          | |
| | [Thinking bubble (absolute above cat, random word/1s)]    | |
| |                                                           | |
| | [Attachment chips: name + X remove]                       | |
| |                                                           | |
| | <textarea auto-grow up to 200px>                          | |
| |                                                           | |
| | [Paperclip] [Search] [ModelPicker] [ModeSelector]  [Stop] [Send] |
| +-----------------------------------------------------------+ |
+---------------------------------------------------------------+
```

### Features

- **Auto-grow textarea**: Grows up to 200px, then scrolls.
- **Submit**: Enter (no Shift) fires `submit()`. Shift+Enter inserts a newline.
- **Staged rewind**: If a rewind is staged (from edit pencil), fires `RewindTo(index)` FIRST, then `Submit(text)`.
- **Steer cap**: Max 5 queued mid-turn submits (shown as preview chips above the card).
- **Thinking bubble**: Shows a random word from `wanderer.json` every 1s while the turn is working.
- **Cat mascot**: Lottie animation, swaps to a random cat on each send.

### File Attachment (images only)

- **Paste**: Ctrl+V → reads clipboard image → `AttachFile` GuiReq.
- **Drag-drop**: Drop image on composer → `AttachFile`.
- **File picker**: Paperclip button → native file dialog → images only → `AttachFile`.
- **Omnisearch pick**: `composerInsert` signal → appends path text to draft (not routed through AttachPath).
- Non-image files are silently skipped.
- Attachment chips appear above the textarea with name + X remove button.
- Remove sends `RemoveAttachment { markerN }`.

---

## 12. Streaming and Token Rendering

### Data Flow

1. Host pushes `StreamMsg { session, text }` where `text` is the **full accumulated response** (not a delta).
2. Store sets `session.stream = env.text` (wholesale replacement).
3. React re-renders `ChatView`, which shows a live `<AssistantMessage streaming>` when `stream.length > 0 || (working && reasoning.trim() !== '')`.
4. `MessageBody` wraps `<Streamdown mode="streaming" parseIncompleteMarkdown={true}>` — handles unterminated markdown gracefully.
5. When the `Snapshot` commit arrives with the message in `messages[]`, React reuses the same DOM node (keyed at `messages.length`) so no flash occurs.

### MessageBody (`MessageBody.tsx`)

- Uses `Streamdown` from the `streamdown` library.
- Per-block memoization to avoid re-rendering completed blocks.
- Code blocks highlighted via `komaCode` plugin (trimmed Shiki, 16 languages, JS regex engine — no WASM).

### komaShiki (`komaShiki.ts`)

Custom Shiki highlighter supporting: TypeScript, TSX, JavaScript, JSX, Python, Rust, Go, Bash, JSON, YAML, TOML, SQL, Markdown, Diff, HTML, CSS.

- Uses `createJavaScriptRegexEngine` (no oniguruma WASM).
- Result cache keyed by code head/tail + length + language (avoids re-highlighting identical blocks).

---

## 13. Tool Call Display

### ToolCallRow (in `ChatView.tsx:125-213`)

Each tool call renders:

```
[pending: Cog spinning] / [done: Check]  toolSignature(args)
  +--- output box (for known tools) ---
```

**Status glyph:**
- Pending → Cog icon (dim, spinning via `animate-spin`).
- Done → Check icon (accent color).

**Signature display:**
- Uses `call.signature` if the host supplies it.
- Otherwise, `fallbackSignature(name, args)` from `lib/toolSignature.ts` — formats as `name(arg)` with smart truncation.

**Plan-ready special case:**
- When `call.name === 'plan_ready'`, the full plan digest is rendered as Markdown with three buttons: Approve / Approve & compact / Chat more.

**Output box (`ToolOutputBox`):**
- For known tools: `bash`, `read`, `grep`, `glob`, `dir_list`, `git_*`, `web_*`, `recall`, `mcp__*`, `sec_*`.
- Renders as a dashed-border card with a family icon + label.
- Up to 5 truncated source lines of output.
- Non-boxed tools get a terse fallback: first non-blank line, truncated to 80 chars.

---

## 14. Approval and Plan Decision

### Tool Approval Overlay (`ApprovalOverlay.tsx`)

**Always mounted**, gated on:
```
awaitingApproval && pendingCall && pendingCall.name !== 'plan_ready'
```

Renders a centered card with:
- ShieldAlert icon + "Approval required" header in warn color.
- Tool signature + classifier reason (if `approvalReason` is set).
- Humanized args: path line, content block (pre-formatted), remaining fields as key-value.
- Deny / Approve buttons → `req({ r: 'ApproveTool', approve: true|false })`.

**When the user clicks:**
- Approve → daemon runs the tool call.
- Deny → daemon bounces the call back to the model (error result).
- Next Snapshot clears `awaitingApproval`/`pendingCall`.

### Plan Decision (inline in ChatView)

When `pendingCall.name === 'plan_ready'`:
- Rendered inline in the `ToolCallRow` (not in the overlay).
- Parses `planDigest(args)` to extract the `highlights` field as Markdown.
- Three buttons: Approve / Approve & compact / Chat more.
- Sends `req({ r: 'PlanDecision', decision: 'approve'|'compact'|'deny' })`.

---

## 15. Accordion System

**`AccordionSection.tsx`** (37 lines) — reusable VSCode-style collapsible section.

### Props

```ts
{
  title: string
  open: boolean
  onToggle: () => void
  action?: ReactNode    // e.g. AddBtn, visible on hover
  children: ReactNode
}
```

### Behavior

- **Open**: `flex-1 min-h-0` — fills remaining parent height and scrolls internally.
- **Closed**: `flex-none` — collapses to just the header.
- **Header**: 22px tall, `bg-koma-head`, hover bg. ChevronRight rotates 90° when open. The `action` slot is hidden until the group is hovered (progressive disclosure).
- **CSS transition**: `transition-transform` on the chevron for smooth rotation.

### Where Used

| AccordionGroup | Sections |
|---------------|----------|
| **ExplorePanel** | Plan, File changed, Bash, Agents |
| **ConnectorListView** | Providers, OAuth, Models |

---

## 16. Explore Panel

**`ExplorePanel.tsx`** (224 lines) — the sidebar's "Explore" view. Four `AccordionSection`s:

### Plan

- **Title**: `Plan · N/M` (completed/total).
- **Items**: `PlanTodoEntry[]` with status icons:
  - Pending → Circle
  - In-progress → CircleDot (accent)
  - Completed → CheckCircle2 (dim + line-through)
  - Cancelled → CircleSlash
- Auto-expands when `mode` flips to `'plan'`.
- Clickable from `UsageFooter`'s PLAN badge via `focusPlanTick`.

### File Changed

- **Title**: `File changed · N`.
- **Items**: `FileChangeEntry[]` with git-style badges:
  - `A` (added) → success/green
  - `M` (modified) → accent
  - `D` (deleted) → error/red + strikethrough
- **Click** → `openDiffTab(path)` — opens a Monaco diff tab.

### Bash

- **Reversed order** (newest first).
- **Items**: `BashJobEntry[]` with terminal icon, command, status badge — includes **foreground** jobs that park the turn as well as true background jobs.
- **Kill button** (while running) → sends `KillBash` with parsed numeric ID.
- Composer / global **Ctrl+B** (`BackgroundAll`) promotes still-blocking FG bash without killing the process.

### Agents

- **Reversed order**.
- **Items**: `SubAgentEntry[]` with bot icon, name, status badge.
- **Kill button** (while running, requires `id`) → sends `KillSubagent`.
- **Background button** / Ctrl+B on a row → `BackgroundSubagent{id}` (detach without kill).

---

## 17. Connector Panel

**`ConnectorPanel.tsx`** — master-detail pattern with AnimatePresence slide animation.

### List View (`ConnectorListView.tsx`)

Three `AccordionSection`s:

#### Providers
- Lists real providers (koma-free hidden from the list).
- Add / Edit (pencil) / Delete (arm-delete: click once to reveal confirm, click again to delete).
- `ProviderForm`: Name, Endpoint, API Key (password field, never prefilled — daemon doesn't send plaintext keys).
- **Marketplace presets** (9): OpenRouter, DeepSeek, Mimo, OpenAI, Groq, Together, Fireworks, Mistral, DeepInfra. Or Custom.
- Selecting a preset auto-fills Name + Endpoint.

#### OAuth
- Shows OpenAI / Kilo Code / Anthropic buttons.
- Stubbed — no backend yet.

#### Models
- Lists real models (free-flagged hidden).
- Add / Edit / Delete.
- `ModelForm`: Name, Provider (Select), Model ID (Combobox with live `ListModels` fetch), Route (radio list with live `ListRoutes` fetch — shows pricing per-million-tokens + uptime %), Roles (Chips: main/awareness/safeguard/compactor/planner), Scope (Select: global/local).

### Config Mutation Flow

1. User action in form → calls `req({ r: 'SetProvider', ... })`.
2. Daemon persists and **re-pushes** the full `Config` envelope.
3. Store replaces `config` slice wholesale.
4. Components re-render from the new config.

This flow works identically for Providers, Models, Themes, and KomaFree — all config mutations follow the same request → persist → Config push → store replace → re-render cycle.

---

## 18. MCP Panel

**`McpPanel.tsx`** — master-detail with slide animation (same pattern as Connector).

### List View (`McpListView.tsx`)

- Server list with enable toggle (green/grey dot), edit (pencil), delete (arm-delete).
- Toggle sends `EnableMcpServer { uuid, enabled }`.

### Edit View (`McpEditView.tsx`)

Form fields:
- Name
- Enabled toggle
- Transport: Segmented control (stdio / http)
- **stdio mode**: Command, Args (space-joined string), Env (`K=V, K2=V2` format)
- **http mode**: URL

New server: `uuid` is null (daemon mints one).
Edit: `uuid` is the existing server's id.

---

## 19. OmniSearch Palette

**`OmniSearchPalette.tsx`** — workspace file search overlay.

### Trigger

- Click the Search icon in the Composer action bar → `openOmniSearch()`.
- Opens as a centered overlay with search input.

### Behavior

1. **Debounced search**: Typing fires `FileSearch { query }` to the daemon.
2. **Results**: Daemon replies with `SearchResults { query, items }` → `session.searchResults`.
3. **Folder drill-in**: Results with a trailing `/` are folders; clicking them prefixes the query.
4. **File pick**: Clicking a file result → `insertToComposer(path)` → appends path text to the Composer textarea.
5. **Close**: Escape or click outside → `closeOmniSearch()`.

### Animation

Framer Motion dropdown: `opacity: 0, y: -4` → `opacity: 1, y: 0` (160ms easeOut, 20ms delay).

---

## 20. Resume Palette and Session Switching

### ResumePalette (`ResumePalette.tsx`)

Opened by clicking the "change session" pill in the Titlebar. Shows:

- **Cooking** (live sessions): Sessions currently attached to a daemon. Shows name, working indicator, foreground marker, directory label.
- **History** (past sessions): Sessions with on-disk data. Shows name, last active time, directory label.
- **[+ new session]** row at the top.

### Session Switch Flow

1. User clicks a session in the palette.
2. React sends `SelectSession { id }` to the host.
3. Host raises `Switching { to }` → store sets `ui.switchingTo` → `SwitchingOverlay` appears.
4. Host detaches from old daemon, attaches to new daemon.
5. Host sends a `Hub` push (clears `switchingTo`) or `Snapshot` (attached).
6. On `Snapshot`, `switchingTo` is always cleared.

### SwitchingOverlay (`SwitchingOverlay.tsx`)

- Full-screen centered spinner with session name.
- "Taking longer than expected…" hint after 10s.
- Auto-cancel at 25s.
- Cancel button sends `CancelSwitch` → host drops to swapper once the target lands.

### RefreshHub

When the ResumePalette is open, it can emit `RefreshHub` requests to re-discover live sessions. The host re-pushes a fresh `Hub` envelope.

---

## 21. Diff Tabs and Monaco Editor

### DiffTab (`DiffTab.tsx`, 282 lines)

Lazy-loaded via `React.lazy()` — the Monaco chunk never loads until the first diff tab is opened.

**States:**
1. **Loading**: Spinner (when `tab.loading === true`).
2. **Error**: Centered error message (when `diff.error` is non-null).
3. **Binary**: "binary file — no preview" notice (when `diff.binary`).
4. **Empty new file**: "New file" notice (when `original === ''`).
5. **Deleted file**: "Deleted" notice (when `modified === ''`).
6. **Diff editor**: Side-by-side Monaco DiffEditor (read-only).

**Monaco integration:**
- `DiffEditor` (side-by-side, read-only) created once per showable tab.
- Models swapped on diff content change — editor persists, only models are recreated (no flash on re-focus).
- **Language detection**: File extension → Monarch tokenizer via `EXT_LANG` map (30+ mappings: ts→typescript, rs→rust, py→python, etc.).
- **Theme**: `applyKomaTheme()` reads live `--color-koma-*` CSS vars, creates a Monaco theme. Applied on each mount.
- **Inlined worker**: Only the base editor worker is inlined via `?worker&inline` — no language workers, no network fetch.

---

## 22. File Attachment Flow

### Image Attachments (drag-drop / paste / file picker)

1. User drops/pastes image on Composer → `Composer.tsx:158-173` → `attachFiles()`.
2. Only images accepted: `if (!file.type.startsWith('image/')) continue`.
3. Read bytes as base64 → `req({ r: 'AttachFile', name, bytesB64, mime })`.
4. Host persists to scratch path, ingests via existing attachment core.
5. Next `Snapshot`: `attachments: AttachmentEntry[]` arrives — full array replaced.
6. Composer shows chips with name + X remove button.
7. ChatView shows `AttachmentCard` under user messages.
8. Remove: Click X → `req({ r: 'RemoveAttachment', markerN })`.

### Path References (omnisearch)

1. User opens omnisearch → `openOmniSearch()`.
2. Typing fires debounced `FileSearch` → `SearchResults` push → `session.searchResults`.
3. User picks file → `insertToComposer(path)` → sets `ui.composerInsert`.
4. Composer effect consumes the signal: appends path text to draft.
5. Clear: `consumeComposerInsert()`.

---

## 23. Onboarding

**`Onboarding.tsx`** (449 lines) — multi-step wizard with progress dots.

### Steps

1. **Theme** (step 1): Grid of theme buttons from `config.themes`. Selecting fires `SetTheme` for live repaint (palette changes propagate instantly via CSS vars).

2. **Connect** (step 2): Three tiles:
   - **Koma Free** (keyless) → sends `SetupKomaFree` → daemon mints provider + model → Config push flips `firstRun` → onboarding unmounts.
   - **Provider** (API key) → opens Provider sub-step.
   - **OAuth** — may be session-gated (greyed until a session exists); Connector
     `OAuthConnect` is a real multi-phase flow, not a placeholder stub.

3. **Pick Provider** (sub-step): Marketplace list from `PREDEFINED` presets + Custom.

4. **Provider Form** (sub-step): `<ProviderForm>` with preset. Auto-advances to Model form after provider saves.

5. **Model Form** (sub-step): `<ModelForm>` pre-seeded with `roles: ['main']`. Saving completes onboarding (the Main model satisfies the gate).

Back/cancel goes one level up. The `useNeedsOnboarding()` gate unmounts onboarding when config has a provider + Main-role model.

### Gate

```ts
function useNeedsOnboarding() {
  const sessionId = useKoma(s => s.session.id)
  const loaded = useKoma(s => s.config.loaded)
  const firstRun = useKoma(s => s.config.firstRun)
  const providers = useKoma(s => s.config.providers)
  const models = useKoma(s => s.config.models)
  const configured = providers.length > 0 && models.some(m => m.roles.includes('main'))
  return loaded && sessionId === null && (firstRun ?? !configured)
}
```

---

## 24. Start Screen

**`StartScreen.tsx`** — shown when no session is attached and onboarding is complete.

- **New session** button → `req({ r: 'NewSession' })`.
- **Recent sessions** list from `hub.history`.
- Clicking a session → `req({ r: 'SelectSession', id })`.
- **About koma** section.
- Uses `useContainerWidth()` hook (ResizeObserver) for responsive layout.

---

## 25. Toasts

### ToastContainer (`ToastContainer.tsx`, 68 lines)

**Source**: Host pushes `Status` envelopes with `toast`/`toastKind` fields.

**Position**: Top-right corner, `z-index: 70`.

**Raise logic** (store):
- Only raises a NEW toast when `env.toast` differs from the current toast text (deduped — the host re-pushes the same live toast on every Status tick).
- A `null` toast from the host does NOT clear an active card (auto-dismiss owns that).
- `toastSeq` bumped on each new toast → ensures re-fired toast gets fresh dismiss timer.

**Auto-dismiss**:
- Error toasts: 7s (`ERROR_MS`).
- Info toasts: 4s (`INFO_MS`).
- Timer keyed on `toast.id` for reset.

**Render**: Framer Motion entry/exit. AlertTriangle icon for errors, Info icon for info. Manual close via X button (only clears if id still matches).

---

## 26. Theme System

### CSS Custom Properties

Set dynamically by `applyPaletteVars()` in `store/koma.ts:604-615` on `document.documentElement.style`:

```css
--koma-bg: #0b0e14      /* Window background */
--koma-fg: #c8d3f5      /* Primary text */
--koma-accent: #39ff14   /* Accent (green neon) */
--koma-dim: #adadad      /* Secondary text */
--koma-panel: #2b2f38    /* User message band / panel bg */
```

### Tailwind v4 Theme Tokens (`styles.css`)

Derived from `--koma-*` vars:

| Token | Derivation |
|-------|-----------|
| `--color-koma-fg` | Direct from `--koma-fg` |
| `--color-koma-bg` | Direct from `--koma-bg` |
| `--color-koma-panel` | `color-mix(fg 6%, bg)` |
| `--color-koma-panel2` | `color-mix(fg 4%, bg)` |
| `--color-koma-border` | `color-mix(fg 10%, bg)` |
| `--color-koma-hover` | `color-mix(fg 8%, bg)` |
| `--color-koma-grip` | `color-mix(fg 22%, bg)` |
| `--color-koma-head` | `color-mix(fg 12%, bg)` |
| `--color-koma-dim` | Direct from `--koma-dim` |
| `--color-koma-accent` | Direct from `--koma-accent` |
| `--color-koma-band` | Direct from `--koma-panel` |
| `--color-koma-warn` | `#ffb43c` (fallback) |
| `--color-koma-success` | `#00c853` (fallback) |
| `--color-koma-info` | `#50c8ff` (fallback) |
| `--color-koma-error` | `#ff3c3c` (fallback) |

Shadcn-compatible tokens are also mapped: `--color-background`, `--color-foreground`, `--color-muted`, `--color-border`, `--color-primary`, etc.

### Live Repainting

The host pushes `PaletteColors` on every `Snapshot` and `Config`. `applyPaletteVars()` sets the `--koma-*` CSS vars on `document.documentElement.style` only when values are valid hex. The Tailwind theme tokens inherit the live values, so palette changes propagate to every component **without re-mounting**.

### Font

**KomaMono** (JetBrains Mono 400/500/700) — the ONE family for everything. Overridden via Tailwind v4 `@theme` block so both `--font-sans` and `--font-mono` resolve to the mono stack. `@font-face` declarations load from `/fonts/jetbrains-mono-{400,500,700}.woff2`.

---

## 27. Animations

### Framer Motion

| Where | Animation |
|-------|-----------|
| **Titlebar cmd bar → ResumePalette/OmniSearchPalette/RenameOverlay** | `layoutId="cmd-search"` / `layoutId="cmd-rename"` with `CMD_SEARCH_SPRING` (stiffness: 450, damping: 50, mass: 0.6) — smooth morph between pill and overlay search bar. |
| **ResumePalette dropdown** | `opacity: 0, y: -4` → `opacity: 1, y: 0` (160ms easeOut, 20ms delay). |
| **OmniSearchPalette** | Same dropdown animation. |
| **RenameOverlay** | `useAnimationControls()` wiggle on outside click: `x: [0, -6, 6, -5, 5, -3, 3, 0]` over 350ms. |
| **SwitchingOverlay** | `opacity: 0, scale: 0.96` → `opacity: 1, scale: 1` (160ms easeOut). |
| **ApprovalOverlay** | `opacity: 0, scale: 0.97, y: 6` → `opacity: 1, scale: 1, y: 0` (160ms easeOut). |
| **ConnectorPanel/McpPanel detail slides** | `AnimatePresence` with `x: '100%'` → `x: 0` → `x: '100%'` (tween 220ms easeOut). |
| **ToastContainer** | `AnimatePresence` enter: `opacity: 0, y: 12, scale: 0.98` → visible; exit: reverse (160ms easeOut). |

### CSS Animations

- `animate-spin` on Loader2 (loading spinners throughout).
- `animate-pulse` on Activity pulse icon, live session dots, CatMascot fallback.
- `transition-transform` on AccordionSection chevron rotation.
- `transition` on various hover states.

### Lottie

4 cat animations in `public/lottie/*.lottie`. Extracted at build time by `vite-plugin-lottie.ts` (unzips dotLottie archives, inlines raster assets as base64 data URIs, exposes parsed JSON as `virtual:lottie-animations`). The `CatMascotLottie` component uses `lottie-react` to render in a loop, swapping to a random cat on each submit.

---

## 28. Keyboard Shortcuts

Keyboard handling is decentralized — there are NO global keyboard shortcut bindings.

| Shortcut | Handler | Where |
|----------|---------|-------|
| **Enter** (Composer textarea, no Shift) | `submit()` | `Composer.tsx:141-146` |
| **Shift+Enter** | Newline (default textarea behavior) | `Composer.tsx:141-146` |
| **Escape** | Close overlays (ResumePalette, RenameOverlay, OmniSearchPalette, ModelPicker, ModeSelector, Select/Combobox) | Each component's `onKey` listener |
| **Enter** (RenameOverlay input) | `confirm()` | `RenameOverlay.tsx:60` |
| **Tab/Space** (tab strip items) | `activateTab()` | `TabBar.tsx:69-71` |

No app-wide Cmd+K, Cmd+P, or chord shortcuts.

---

## 29. Component Inventory

| Component | File | Purpose |
|-----------|------|---------|
| `AccordionSection` | `AccordionSection.tsx` | Reusable VSCode-style collapsible section |
| `ActivityBar` | `ActivityBar.tsx` | 48px icon strip (Explore/MCP/Connector + Settings) |
| `ApprovalOverlay` | `ApprovalOverlay.tsx` | Risky tool call approval card |
| `CatMascot` | `CatMascot.tsx` | Persistent cat mascot with error boundary + lazy Lottie |
| `CatMascotLottie` | `CatMascotLottie.tsx` | Lottie player leaf, random cat swap on trigger |
| `ChatView` | `ChatView.tsx` | Full chat: messages, streaming, tools, reasoning, composer, footer |
| `Composer` | `Composer.tsx` | Message input, attachments, model/mode pickers, thinking bubble |
| `DiffTab` | `DiffTab.tsx` | Monaco DiffEditor (lazy), language detection, live theme |
| `MessageBody` | `MessageBody.tsx` | Streamdown markdown renderer with Shiki code highlighting |
| `ModeSelector` | `ModeSelector.tsx` | Auto/Plan/Normal mode dropdown (opens upward) |
| `ModelPicker` | `ModelPicker.tsx` | Session model quick-picker dropdown (opens upward) |
| `OmniSearchPalette` | `OmniSearchPalette.tsx` | Workspace file search overlay with folder drill-in |
| `Onboarding` | `Onboarding.tsx` | First-run 3-step wizard (Theme → Connect → Model) |
| `RenameOverlay` | `RenameOverlay.tsx` | Session rename input (wiggle on outside click) |
| `ResizeHandles` | `ResizeHandles.tsx` | 8 custom window resize handles (frameless window) |
| `ResumePalette` | `ResumePalette.tsx` | Session switcher overlay (cooking + history, filtered) |
| `Sidebar` | `Sidebar.tsx` | Shell: header + Explore/MCP/Connector panel |
| `StartScreen` | `StartScreen.tsx` | Pre-session landing with recent sessions + New |
| `SwitchingOverlay` | `SwitchingOverlay.tsx` | Full-screen loading spinner during session swap |
| `TabBar` | `TabBar.tsx` | VSCode-style tab strip (chat + diff tabs) |
| `Titlebar` | `Titlebar.tsx` | Custom frameless titlebar with cmd bar, window controls |
| `ToastContainer` | `ToastContainer.tsx` | Animated toast notifications (error/info, auto-dismiss) |
| `UsageFooter` | `UsageFooter.tsx` | Statusline: mode badge, activity pulse, token counts, compact btn |
| `komaShiki` | `komaShiki.ts` | Trimmed Shiki highlighter (16 langs, JS engine, no WASM) |
| `ExplorePanel` | `panels/ExplorePanel.tsx` | Plan todos, File changes, Bash jobs, Sub-agents |
| `ConnectorPanel` | `panels/ConnectorPanel.tsx` | Provider/Model/OAuth management |
| `McpPanel` | `panels/McpPanel.tsx` | MCP server management |
| `ConnectorListView` | `panels/connector/ConnectorListView.tsx` | 3-accordion list |
| `ProviderForm` | `panels/connector/ProviderForm.tsx` | Provider create/edit with marketplace presets |
| `ModelForm` | `panels/connector/ModelForm.tsx` | Model create/edit with live route picker |
| `OAuthConnect` | `panels/connector/OAuthConnect.tsx` | OAuth connect flow (OpenAI / Kilo / Anthropic / …) |
| `CodingPanel` | `panels/CodingPanel.tsx` | Workspace file tree |
| `CodeEditorTab` | `CodeEditorTab.tsx` | Monaco coding tabs + host LSP |
| `GitPanel` | `panels/GitPanel.tsx` | Source control |
| `TerminalTab` | `TerminalTab.tsx` | Integrated terminal tabs |
| `ProblemsDrawer` / `LspDrawer` | drawers | Diagnostics + language-server runtime |
| `McpListView` | `panels/mcp/McpListView.tsx` | MCP server list with toggle/delete |
| `McpEditView` | `panels/mcp/McpEditView.tsx` | MCP server create/edit form |
| `form.tsx` | `panels/form.tsx` | Form primitives: Field, TextInput, Toggle, Segmented, Chips, Select, Combobox |
| `helpers.tsx` | `panels/helpers.tsx` | Row, IconBtn, Empty, ScopePill, AddBtn, DetailHeader, FormActions |

---

## 30. Key Constants and Limits

| Constant | Value | Where |
|----------|-------|-------|
| Default window size | 1024×680 | `gui/mod.rs` WindowBuilder |
| Titlebar height | 32px | `#titlebar` CSS |
| ActivityBar width | 48px | `w-12` class |
| Sidebar min/max | 150–500px | `SIDEBAR_MIN`/`SIDEBAR_MAX` |
| Chat max-width | 1024px | `.term-shell` CSS |
| Textarea max-height | 200px | Composer auto-grow |
| Steer queue cap | 5 | Composer |
| Toast auto-dismiss (error) | 7s | `ToastContainer.tsx` |
| Toast auto-dismiss (info) | 4s | `ToastContainer.tsx` |
| SwitchingOverlay hint | 10s | `SwitchingOverlay.tsx` |
| SwitchingOverlay auto-cancel | 25s | `SwitchingOverlay.tsx` |
| Monaco chunk | lazy-loaded | `React.lazy()` |
| Shiki languages | 16 | `komaShiki.ts` |
| Scrollbar width | 3px | `::-webkit-scrollbar` |
| `#app` border-radius | 10px (16px macOS) | `styles.css` |
| Cat mascot swap | Per submit | `CatMascotLottie.tsx` |
| Thinking bubble interval | 1s | Composer |
