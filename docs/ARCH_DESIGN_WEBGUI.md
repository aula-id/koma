# Architecture: Web GUI (`src-webgui`)

> For the core agent/TUI architecture, see [`ARCHITECTURE.md`](ARCHITECTURE.md).

A React 19 single-page application serving as the graphical interface for koma.
It runs inside a transparent, frameless window (tao) and communicates with the
Rust agent host exclusively through an injected JavaScript bridge — no WebSocket,
no HTTP fetch, no REST API.

---

## 1. Tech Stack

| Layer | Technology |
|---|---|
| Framework | React 19 (strict mode) |
| Routing | TanStack Router v1.87 (hash history) |
| State | Zustand v5 (single store, no middleware) |
| Styling | Tailwind CSS v4 (`@theme` block, CSS custom properties) |
| Animation | Framer Motion v12 (shared layout morphs, overlays, toasts) |
| Icons | lucide-react |
| Markdown | Streamdown (Vercel) — streaming-safe, per-block memoized |
| Syntax Highlighting | Custom Shiki plugin (`komaShiki.ts`) — 16 languages, JS regex engine (no WASM) |
| Diff Editor | Monaco (lazy); Monarch tokenizers on **diff** path |
| Code editor | Monaco coding tabs + **host LSP** (`monaco-lsp`, `src-agent/src/lsp/`) |
| Lottie | lottie-react + custom Vite plugin (build-time dotLottie extraction, no WASM) |
| Fonts | JetBrains Mono (3 weights, 400/500/700) as "KomaMono" — the only font family |
| Build | Vite 6, TypeScript 5.7, `@vitejs/plugin-react`, `@tailwindcss/vite` |
| Tests | Vitest 4: `logic` project (jsdom, `*.vitest.test.ts(x)`) and `chromium` project (Playwright browser mode, `*.browser.test.ts(x)`) |

---

## 2. Architecture Overview

```
┌──────────────────────────────────────────────────────────────┐
│  tao (transparent, frameless window)                         │
│                                                              │
│  ┌─ Titlebar ────────────────────────────────────────────┐   │
│  │  session pill · rename · window controls · drag        │   │
│  └───────────────────────────────────────────────────────┘   │
│  ┌─ ActivityBar ─┬─ Sidebar ─────────┬─ main ────────────┐  │
│  │  [Chat]        │  ExplorePanel     │  <Outlet>         │  │
│  │  [Coding]      │  CodingPanel      │   ├─ Onboarding   │  │
│  │  [Source Ctl]  │  GitPanel         │   └─ TabbedMain   │  │
│  │  [MCP] …       │  …                │       ├─ TabBar    │  │
│  │  footer: Help  │                   │       ├─ Chat /   │  │
│  │  Settings      │                   │       │  Code /   │  │
│  │                │                   │       │  Term /…  │  │
│  │                │                   │       └─ DiffTab   │  │
│  │                │                   │  BottomPanel       │  │
│  │                │                   │  UsageFooter       │  │
│  └────────────────┴───────────────────┴───────────────────┘  │
│  ┌─ Overlays (absolute/portaled) ────────────────────────┐   │
│  │  ResumePalette · OmniSearch · SwitchingOverlay · Toasts │  │
│  │  ResizeHandles · RenameOverlay                          │  │
│  └───────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────┘
```

The layout follows a VSCode-style chrome pattern: a narrow icon strip (ActivityBar)
drives which panel is shown in the Sidebar, and the main content area renders the
active route. The window is undecorated; custom resize handles and titlebar buttons
replace the OS chrome.

---

## 3. IPC Bridge Protocol

All communication flows through injected globals — the Rust host (tao/wry) sets
these before the app boots:

| Global | Direction | Purpose |
|---|---|---|
| `window.ipc.postMessage(json)` | JS → Rust | Requests and window-chrome commands |
| `window.__komaClient.push(json \| object)` | Rust → JS | Authoritative state pushes |
| `window.__komaClient.pushBatch(items)` | Rust → JS | All pushes of one display-frame window in one call, wire order preserved |
| `window.__komaOS` | Rust → JS | OS detection (`macos`, `linux`, `windows`) |
| `window.__komaOpaque` | Rust → JS | Host window is opaque (square `#app` corners) |
| `window.komaIpc(req)` | JS → Rust | Fire-and-forget wrapper that posts `{ t: 'req', ...req }` |

### JS → Rust: `GuiReq`

```typescript
window.ipc.postMessage(JSON.stringify({ t: 'req', r: '<kind>', ...params }))
```

The full `GuiReq` union is declared in `koma.d.ts` (well over a hundred
request kinds: chat, hub, config, MCP, skills, git, coding, LSP, remote,
notifications and more), including: `Ready`, `Submit`, `SelectSession`,
`NewSession`, `RefreshHub`, `AttachFile`, `FileSearch`, `SetMcpServer`,
`SetProvider`, `SetModel`, `SetMode`, `Interrupt`, `RewindTo`, `KillSubagent`,
`KillBash`, `ApproveTool`, `PlanDecision`, `Compact`, `FileDiff`. The
per-request reference lives in [`WEBGUI_IPC.md`](WEBGUI_IPC.md).

Requests are fire-and-forget serialisations. The store's `req()` helper
(`store/koma.ts`) posts them as `{ t: 'req', ...g }` after tagging the
notification origin; coding file requests (`FileTree`, `FileRead`, …) are
routed through the active remote host view when one is selected.

### Rust → JS: `PushEnvelope`

```typescript
window.__komaClient.push(json)  // Rust calls via evaluate_script
```

The `PushEvent` union (`store/types/envelope.ts`, discriminant field `k`) has
roughly ninety variants, including `Snapshot`, `Switching`, `StreamMsg`,
`StreamDelta`, `Reasoning`, `Status`, `Hub`, `SearchResults`, `Config`,
`ModelList`, `RouteList`, `FileDiff`, `Notifications`, `SkillValues`,
`McpStatus`, `RepoList`, `GitStatus`, `LspDiagnostics`. Handlers are split by
domain under `store/push/` (`session`, `config`, `git`, `coding`, `skills`,
`agents`, `marketplace`, `importGraph`, `analytics`, `computer`).

Each push replaces the relevant state slice wholesale — the GUI never
accumulates state from pushes.

### Window Chrome Messages

```typescript
// Titlebar / ResizeHandles
window.ipc.postMessage(JSON.stringify({ t: 'win', a: 'drag' }))     // drag
window.ipc.postMessage(JSON.stringify({ t: 'win', a: 'min' }))     // minimize
window.ipc.postMessage(JSON.stringify({ t: 'win', a: 'max' }))     // maximize
window.ipc.postMessage(JSON.stringify({ t: 'win', a: 'close' }))   // close
window.ipc.postMessage(JSON.stringify({ t: 'winresize', dir: 'n' })) // resize
```

---

## 4. State Management

### Single Zustand Store

All state lives in one Zustand store (`src/store/koma.ts`; state types in
`store/state.ts` and `store/types/`, actions under `store/actions/`, push
reducers under `store/push/`). The core slices are:

| Slice | Shape | Contents |
|---|---|---|
| `session` | `SessionSlice` | id, messages, streaming buffer, reasoning buffer, subagents, bash jobs, fileChanges, planTodos, attachments, mode, pendingSteer, approval gate, usage counters |
| `hub` | `HubSlice` | `cooking` (live session) + `history` (past sessions) rows |
| `config` | `ConfigSlice` | MCP servers, providers, models, firstRun flag, theme registry |
| `ui` | `UiSlice` | Local-only: omnisearch, composer signals, rewind staging, scroll tick, switching overlay, toast, editor tabs and groups, focus plan tick |
| `palette` | `PaletteColors` | Live theme: bg/fg/accent/dim/hex values |

Further slices cover `oauth`, `store` (extension marketplace), `tutorial`, `graph`, `activity`, `analytics`, `importGraph`, `coding`, `diagram`, `design`, plus `activityBar` (order/hidden icons) and `remoteState`.

### Host-Authoritative State Model

The GUI **never accumulates state locally**. Every `PushEnvelope` replaces the
relevant slice wholesale. The GUI is a pure projection of the Rust host's state.

```
Rust host push → push() reducer → replaces slice → React re-render
JS request    → req() helper   → ipc.postMessage → Rust processes → push back
```

Components select narrow slices for fine-grained subscriptions:

```typescript
const messages = useKoma((s) => s.session.messages)
const stream  = useKoma((s) => s.session.stream)
```

### One-Shot Signal Coordination

Cross-component communication without prop drilling uses store-level one-shot
signals with explicit consume/ack patterns:

| Signal | Producer | Consumer | Purpose |
|---|---|---|---|
| `composerInsert` | OmniSearchPalette | Composer | Insert picked file path |
| `consumeComposerInsert` | Composer | — | Ack insertion |
| `composerRefill` | ResumePalette (rewind) | Composer | Refill with old message |
| `consumeComposerRefill` | Composer | — | Ack refill |
| `pendingRewindIndex` | ChatView edit pencil | Composer send | Pre-fill rewind index |
| `clearRewind` | Composer send | — | Ack rewind |
| `scrollTick` | Composer send | ChatView | Force jump-to-bottom |
| `focusPlanTick` | UsageFooter PLAN badge | RootLayout / ExplorePanel | Open the Chat sidebar view and expand its plan section |

---

## 5. Component Hierarchy

```
<RouterProvider>                              (main.tsx)
  └─ <RootLayout>                            (routes/index.tsx)
       ├─ <Titlebar>                         — frameless window chrome
       ├─ <ActivityBar>                      — VSCode-style icon strip (reorderable, overflow menu)
       ├─ <Sidebar>                          — panel container, one panel per SidebarView
       │    ├─ <ExplorePanel>                — view id 'explore', titled "Chat": Plan/Todo / Files changed / Bash / Agents
       │    ├─ <CodingPanel> · <GitPanel>    — Coding · Source Control
       │    ├─ <McpPanel>                    — MCP server list / edit / JSON import
       │    ├─ <ConnectorPanel>              — Provider / OAuth / Model CRUD
       │    └─ AgentsPanel, SkillsPanel, UsagePanel, ImportGraphPanel,
       │       StorePanel (Extensions), RemotePanel, DiagramPanel, DesignPanel
       ├─ <main> → <Outlet>
       │    └─ <IndexPage>                   — route content
       │         ├─ <Onboarding>             — first-run wizard (Theme → Connect → Model)
       │         └─ <TabbedMain>             — editor groups and tabs
       │              ├─ <TabBar>            — permanent chat tab + closable tabs (diff, file, terminal, skill, Welcome, …)
       │              ├─ <StartScreen>        — chat slot when no session is attached (new + recent)
       │              ├─ <ChatView>          — message list + streaming (session attached)
       │              │    ├─ <Message>       — UserMessage / AssistantMessage / ShellMessage
       │              │    │    ├─ <MessageBody>         — Streamdown markdown
       │              │    │    ├─ <ToolCallRow>          — tool call + inline result
       │              │    │    ├─ <ReasoningBlock>       — collapsible thinking
       │              │    │    └─ <AttachmentCard>       — image attachments
       │              │    ├─ <ApprovalOverlay>          — risky tool gate
       │              │    ├─ <Composer>                 — input + controls
       │              │    │    ├─ <CatMascot>            — decorative lottie cat
       │              │    │    ├─ <ModelPicker>          — session model quick-switch
       │              │    │    ├─ <EffortPicker>
       │              │    │    └─ <ModeSelector>         — Auto / Plan / Normal
       │              └─ <DiffTab> and other tab kinds (see store/types/tabs.ts)
       ├─ <BottomPanel>                      — Tasks / Problems / Language Servers / Language Packs dock
       ├─ <UsageFooter>                      — statusline
       ├─ <ResumePalette>                    — session switcher overlay
       ├─ <RenameOverlay>                    — rename input overlay
       ├─ <OmniSearchPalette>                — workspace file search overlay
       ├─ <SwitchingOverlay>                 — full-screen loader during session swap
       ├─ <AboutDialog>                      — About Koma (build info)
       ├─ <ToastContainer>                   — transient toast surface
       └─ <ResizeHandles>                    — 8 edge/corner drag zones
```

---

## 6. Theming

### CSS Custom Properties

The palette is driven entirely by the Rust host. Every `Snapshot` or `Config`
push carries a `PaletteColors` object; the store's `applyPaletteVars()` writes
these to `document.documentElement.style`:

```css
/* styles.css — @theme block maps Tailwind utilities to CSS vars */
--koma-bg     /* background */
--koma-fg     /* foreground text */
--koma-accent /* primary accent */
--koma-dim    /* dimmed text */
--koma-panel  /* panel background */

/* Derived tokens via color-mix() */
--koma-panel2, --koma-border, --koma-hover, --koma-grip, --koma-head

/* Semantic roles (fallback hex, overridden by host) */
--koma-warn, --koma-success, --koma-info, --koma-error

/* shadcn-compatible aliases (for Streamdown) */
--color-background, --color-foreground, --color-muted, etc.
```

### Font

Single monospace family "KomaMono" (JetBrains Mono) at weights 400/500/700.
Overrides both `--font-sans` and `--font-mono` in Tailwind's theme, so the
entire GUI is monospaced.

### Window Chrome

The Rust host creates a transparent, frameless window via tao. The `#app` div
supplies the visible rounded canvas (10px radius, 16px on macOS). macOS
traffic-light layout is handled via `.os-macos#app` CSS.

---

## 7. Routing

TanStack Router with hash history (required for the `koma://` custom protocol):

```typescript
// routes/index.tsx
export const routeTree = rootRoute.addChildren([indexRoute, settingsRoute])
// router.tsx: createRouter({ routeTree, history: createHashHistory() })
```

`routes/index.tsx` registers `/` and a stub `/settings` route (placeholder
text; the real settings UI is `SettingsTab`). The `IndexPage` renders
`Onboarding` (first run) or `TabbedMain`; `TabbedMain` shows `StartScreen` in
its chat slot while no session is attached. All navigation is state-driven via
the store, not URL-driven.

---

## 8. Build & Protocol Constraints

### WASM-Free Design

The entire GUI avoids WASM because the `koma://` custom protocol cannot
reliably serve WASM blobs. This constraint shapes several key decisions:

| Component | WASM-free approach |
|---|---|
| Shiki (syntax highlight) | JS regex engine instead of oniguruma; 16 trimmed languages |
| Monaco (diff editor) | Inlined base worker; Monarch tokenizers (no LSP on diff path) |
| Monaco (coding) | Host LSP via `monaco-lsp` + `src-agent/src/lsp/` |
| Lottie (animations) | `lottie-react` (pure JS) instead of `@lottiefiles/dotlottie-react` |

### Custom Vite Plugin: Lottie

`vite-plugin-lottie.ts` runs at build time:

1. Reads each `.lottie` file (a ZIP archive)
2. Extracts the inner Lottie JSON
3. Inlines external raster assets as base64 data URIs
4. Exports everything via the `virtual:lottie-animations` virtual module

This avoids shipping a WASM dotLottie reader in the runtime.

### Monaco

Lazy-loaded. Diff tabs use Monarch tokenizers only (no Monaco language-server
workers). **Coding** tabs attach **host-spawned LSP** via `monaco-lsp.ts` /
`lsp-bridge.ts` and `src-agent/src/lsp/` (completion, hover, definition,
diagnostics, …). Theme follows live `--color-koma-*` CSS vars.

### Shiki Code Highlighting

`komaShiki.ts` maintains a trimmed Shiki highlighter:

- ~16 languages with lazy dynamic imports per grammar
- JavaScript regex engine (no oniguruma WASM)
- `github-dark` and `github-light` themes
- Result cache keyed by content head/tail + length + language

### Build Output

```typescript
// vite.config.ts
base: './'  // relative paths for koma:// protocol
build: { outDir: 'dist', emptyOutDir: true }
```

---

## 9. Key Design Patterns

### Host-Authoritative State

The GUI never accumulates state locally. Every `PushEnvelope` replaces the
relevant slice wholesale. The GUI is a pure projection of the Rust host's state.

### One-Shot Signal Coordination

Cross-component communication without prop drilling uses store-level one-shot
signals with explicit consume/ack patterns (see Section 4).

### VSCode-Style Chrome

ActivityBar → Sidebar → panel pattern. `AccordionSection` with auto-fill sizing and an optional `count`.
`TabBar` with a permanent chat tab + closable tabs. Armed delete confirmation
pattern (Row component).

### Shared Layout Morphs

Framer Motion `layoutId` for smooth morphs between:
- Titlebar session pill ↔ ResumePalette search bar
- Titlebar rename button ↔ RenameOverlay input

Shared spring config: `CMD_SEARCH_SPRING`.

### Master-Detail with Slide Transitions

`ConnectorPanel` and `McpPanel` use `AnimatePresence` with
`x: '100%'` → `x: 0` slide for list → detail navigation (`McpPanel` also
slides in `McpImportView` for JSON import).

### Lazy Loading

- `DiffTab` (and other heavy tabs such as Settings, Help, Skill): separate chunks via `React.lazy`
- `CatMascotLottie`: lottie-react player + animation JSON (separate chunk)
- `komaShiki.ts`: per-language grammar dynamic imports

### Error Boundaries

`MascotBoundary` (CatMascot.tsx): class-based error boundary isolating
Lottie runtime failures, degrading to a CSS pulse animation.

### Portal-Based Dropdowns

`Select` and `Combobox` (form.tsx) render their menus via `createPortal` to
`document.body` at `z-[80]`, with anchor rect tracking to survive
scroll/resize. Avoids clipping by overflow ancestors.

### Optimistic UI with Deterministic Recovery

`SwitchingOverlay` is raised optimistically (no host "swap started" push),
cleared by:
- Next `Snapshot` (success)
- Next `Hub` push (failure — host always bounces back to swapper)
- Auto-cancel after 25 s (last-resort trap escape)

### Defensive Backward Compatibility

Every push envelope field is treated as optional-tolerant — newer fields carry
`?? fallback` so an older host build that doesn't project them yet doesn't
crash the GUI. Applied consistently across the store push handlers.

---

## 10. File Map

```
src-webgui/
├── index.html                         SPA shell (<div id="root">)
├── package.json                       Dependencies and scripts (dev, build, test, test:logic, test:browser)
├── vitest.config.ts                   `logic` (jsdom) and `chromium` (browser) test projects
├── tsconfig.json                      TypeScript config (ES2020, strict, Monaco alias)
├── vite.config.ts                     Vite 6 + React + Tailwind + Lottie plugins
├── vite-plugin-lottie.ts              Build-time dotLottie extraction plugin
├── public/
│   ├── fonts/                         KomaMono (JetBrains Mono woff2)
│   └── lottie/                        dotLottie archives (3 files; the plugin uses two)
└── src/
    ├── main.tsx                        ReactDOM root + RouterProvider
    ├── router.tsx                      TanStack Router (hash history; `/` and a stub `/settings`)
    ├── styles.css                      Tailwind @theme, @font-face, markdown overrides
    ├── koma.d.ts                       Ambient types: GuiReq, KomaClient, Window
    ├── vite-env.d.ts                   Vite + virtual module declarations
    ├── store/
    │   ├── koma.ts                     Single Zustand store (push dispatcher, req helper)
    │   ├── state.ts, initial.ts        Store state type and initial slices
    │   ├── actions/                    Per-domain actions (session, git, skills, tabs, …)
    │   ├── push/                       Per-domain PushEnvelope reducers
    │   └── types/                      Envelope, slice, tab and domain types
    ├── types/
    │   └── config.ts                   Shared types: Provider, Model, McpServer, etc.
    ├── lib/                            Non-UI logic (toolSignature.ts, skillUpload.ts, notifications.ts, lsp-bridge.ts, …)
    ├── utils/                          logError.ts
    ├── routes/
    │   └── index.tsx                   RootLayout + IndexPage (Onboarding/TabbedMain)
    └── components/                     (excerpt; one file per component, tests alongside)
        ├── ChatView.tsx                Message list + live streaming + scroll-stick
        ├── Composer.tsx                Input textarea + attach + send + model picker + mode selector
        ├── MessageBody.tsx             Streamdown markdown renderer + Shiki plugin
        ├── komaShiki.ts                Trimmed Shiki highlighter (~16 langs, JS regex)
        ├── Titlebar.tsx                Frameless titlebar: session pill, rename, window controls
        ├── ActivityBar.tsx             VSCode-style icon strip (ACTIVITY_BAR_ITEMS, overflow menu)
        ├── Sidebar.tsx                 Panel container (routes to active panel)
        ├── TabBar.tsx                  Tab strip: permanent chat tab + closable tabs
        ├── StartScreen.tsx             Pre-session landing (new + recent)
        ├── BottomPanel.tsx             Bottom dock: Tasks / Problems / Language Servers / Language Packs; resizable, maximisable
        ├── PanelChrome.tsx             Shared panel body/empty helpers and header-action slot for BottomPanel
        ├── AboutKoma.tsx               About Koma dialog (build info from lib/buildInfo.ts)
        ├── WelcomeTab.tsx              Welcome tab for Diagram and Design
        ├── Onboarding.tsx              First-run wizard (Theme → Connect → Model)
        ├── ModelPicker.tsx             Composer quick-picker for session model
        ├── ModeSelector.tsx            Auto / Plan / Normal dropdown
        ├── ApprovalOverlay.tsx         Risky tool approval modal
        ├── RenameOverlay.tsx           Titlebar rename input (layoutId morph)
        ├── ResumePalette.tsx           Session switcher overlay (search + lists)
        ├── OmniSearchPalette.tsx       Workspace file search overlay (fuzzy)
        ├── SwitchingOverlay.tsx        Full-screen loader during session swap
        ├── ToastContainer.tsx          Transient toast surface (framer-motion)
        ├── UsageFooter.tsx             Statusline: mode, tokens, cost, compact
        ├── ResizeHandles.tsx           8 edge/corner drag zones
        ├── AccordionSection.tsx        Collapsible section with optional action and count
        ├── CatMascot.tsx               Lazy cat mascot + error boundary
        ├── CatMascotLottie.tsx         Lottie player for cat animations
        ├── DiffTab.tsx                 Lazy Monaco DiffEditor with live theming
        └── panels/
            ├── ExplorePanel.tsx         "Chat" view: Plan/Todo / Files changed / Bash / Agents accordion
            ├── ConnectorPanel.tsx       Provider / OAuth / Model CRUD
            ├── McpPanel.tsx             MCP server list / edit / JSON import
            ├── CodingPanel, GitPanel, AgentsPanel, SkillsPanel, UsagePanel,
            │   ImportGraphPanel, StorePanel, RemotePanel, DiagramPanel, DesignPanel
            ├── form.tsx                 Reusable form primitives (Field, Toggle, Select, etc.)
            ├── helpers.tsx              Shared UI atoms (Row, IconBtn, Empty, EmptyState, NoProjectState, NoWorkspaceState, AddBtn, …)
            ├── connector/
            │   ├── ConnectorListView.tsx  Provider/model accordion list
            │   ├── ProviderForm.tsx        Provider form + marketplace picker
            │   ├── ModelForm.tsx           Model form + catalogue combobox
            │   └── OAuthConnect.tsx        OAuth connect flow (OpenAI / Kilo / Anthropic / …)
            └── mcp/
                ├── McpListView.tsx          MCP server list with toggle/restart/edit/delete
                ├── McpEditView.tsx          MCP server form (transport, command, URL, env, headers)
                ├── McpImportView.tsx        JSON import of Claude Code / Cursor / VS Code snippets
                └── mcpForm.ts               Draft <-> request conversion
```
