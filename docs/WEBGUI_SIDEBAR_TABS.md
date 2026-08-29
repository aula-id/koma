# Web GUI: Adding a Sidebar Menu and Tab

This guide describes how to add a new ActivityBar menu item, its Sidebar panel,
and a tab that the panel can open or focus.

The GUI uses two separate UI layers:

```text
ActivityBar menu item
  -> RootLayout active SidebarView
  -> Sidebar panel
  -> Zustand store action
  -> ui.tabs + ui.activeTabId
  -> TabBar
  -> TabbedMain tab content
```

A Sidebar menu controls which panel is visible. A tab is editor-style content in
the main column. Do not connect these with component refs or prop drilling; use
the existing Zustand store action as the boundary.

## Relevant files

| Purpose | File |
|---|---|
| ActivityBar menu items | `src-webgui/src/components/ActivityBar.tsx` |
| Sidebar view type and panel routing | `src-webgui/src/components/Sidebar.tsx` |
| New Sidebar panel | `src-webgui/src/components/panels/<Name>Panel.tsx` |
| Tab state and open/close actions | `src-webgui/src/store/koma.ts` |
| Tab strip rendering | `src-webgui/src/components/TabBar.tsx` |
| Main tab content routing | `src-webgui/src/routes/index.tsx` |
| New tab content | `src-webgui/src/components/<Name>Tab.tsx` |
| Rust bridge request types, if needed | `src-webgui/src/koma.d.ts` and the Rust GUI host |

## 1. Add the Sidebar view

In `Sidebar.tsx`, add a literal to `SidebarView`, add its title, import the
panel, and render it in the existing view switch.

```tsx
import { TasksPanel } from './panels/TasksPanel'

export type SidebarView =
  | 'explore'
  | 'coding'
  | 'git'
  | 'mcp'
  | 'connector'
  | 'importGraph'
  | 'agents'
  | 'usage'
  | 'store'
  | 'remote'

const TITLES: Record<SidebarView, string> = {
  explore: 'Explorer',
  coding: 'Coding',
  git: 'Source Control',
  mcp: 'MCP Servers',
  connector: 'Connector',
  importGraph: 'Import Graph',
  agents: 'Agents',
  usage: 'Usage',
  store: 'Extensions',
  remote: 'Remote',
}
```

Add the panel beside the other panel branches:

```tsx
{view === 'tasks' && <TasksPanel />}
```

(You must also extend `SidebarView` / `TITLES` with your new literal — the example
above shows the **current** built-in set; a new view is an additive union member.)

`RootLayout` already stores `activeView` and handles the standard behavior:
clicking the active icon collapses the Sidebar; clicking another icon selects
that view and opens the Sidebar. No RootLayout change is required for a normal
new menu item.

## 2. Add the ActivityBar menu item

In `ActivityBar.tsx`, import a suitable `lucide-react` icon and append an item
to `ACTIVITY_BAR_ITEMS`:

```tsx
import { ListTodo } from 'lucide-react'

export const ACTIVITY_BAR_ITEMS: ActivityBarItem[] = [
  // existing items...
  { view: 'tasks', icon: ListTodo, label: 'Tasks' },
]
```

`ACTIVITY_BAR_ITEMS` uses the same `SidebarView` literal, so TypeScript catches a menu item
that does not have a matching Sidebar view.

## 3. Create the Sidebar panel

Create `src-webgui/src/components/panels/TasksPanel.tsx`. A panel button opens
the main-column tab through the store:

```tsx
import { ListTodo } from 'lucide-react'
import { useKoma } from '../../store/koma'

export function TasksPanel() {
  const openTasksTab = useKoma((s) => s.openTasksTab)

  return (
    <div className="flex h-full flex-col">
      <button
        onClick={openTasksTab}
        className="flex items-center gap-2 px-3 py-2 text-left text-[12px] text-koma-fg hover:bg-koma-hover"
      >
        <ListTodo size={14} />
        <span>Open Tasks</span>
      </button>
    </div>
  )
}
```

Keep panel-local interaction and display state in the panel when it does not
need to coordinate with another component. Use the store for state shared with
the tab or the host bridge.

## 4. Add the tab to the Zustand store

For a singleton tab, add a fixed ID and a new `kind` to the `Tab` union in
`src-webgui/src/store/koma.ts`:

```tsx
export type Tab =
  | { id: 'chat'; kind: 'chat' }
  | { id: 'tasks'; kind: 'tasks' }
  // existing variants...
```

Add the action to `KomaState`:

```tsx
openTasksTab: () => void
```

Implement it beside `openSettingsTab` and `openHelpTab`:

```tsx
openTasksTab: () => {
  set((s) => {
    const exists = s.ui.tabs.some((t) => t.id === 'tasks')
    const tabs: Tab[] = exists
      ? s.ui.tabs
      : [...s.ui.tabs, { id: 'tasks', kind: 'tasks' }]

    return {
      ui: {
        ...s.ui,
        tabs,
        activeTabId: 'tasks',
      },
    }
  })
},
```

This is an open-or-focus action. Repeated clicks must not create duplicate
singleton tabs.

### Singleton versus document tabs

Use a fixed ID when there should be one tab for the feature:

```text
settings
help
graph
tasks
```

Use a stable, data-derived ID when users can open multiple instances, such as
file diffs:

```text
diff:<path>
gitdiff:<staged|unstaged>:<path>
```

If a tab's identity can change after editing, keep its React identity stable
and update its data key separately. The agent editor tabs follow this pattern
in `koma.ts`.

## 5. Add the tab content

Create `src-webgui/src/components/TasksTab.tsx`:

```tsx
import { ListTodo } from 'lucide-react'

export function TasksTab() {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex items-center gap-2 border-b border-koma-border px-4 py-3">
        <ListTodo size={16} />
        <h1 className="text-[13px] font-semibold text-koma-fg">Tasks</h1>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto p-4 text-[13px] text-koma-dim">
        Task content goes here.
      </div>
    </div>
  )
}
```

Replace the placeholder with the feature UI. Use `min-h-0 flex-1 overflow-y-auto`
for scrollable tab content so it fits inside `TabbedMain`.

## 6. Render the tab from `TabbedMain`

Import the tab in `src-webgui/src/routes/index.tsx`:

```tsx
import { TasksTab } from '../components/TasksTab'
```

Add a branch to the `tabs.map(...)` expression in `TabbedMain`:

```tsx
) : t.kind === 'tasks' ? (
  <div key={t.id} className={`absolute inset-0 ${activeTabId === t.id ? '' : 'hidden'}`}>
    <TasksTab />
  </div>
) : null
```

Inactive tabs are hidden rather than unmounted. This preserves local component
state while the user switches between tabs. Close the tab through the existing
`closeTab` action; do not add a second close implementation in the tab content.

## 7. Add the tab strip entry

In `TabBar.tsx`, add an icon import and a closeable tab branch. The existing
Settings, Help, Graph, Agent, and stream branches are good templates.

The branch should:

- call `activateTab(t.id)` when the tab body is clicked;
- call `closeTab(t.id)` from its close button;
- stop propagation on the close button;
- use the shared `base`, `tone`, and active `accent` styling;
- support Enter and Space when the tab body has `role="button"`.

Example shape:

```tsx
if (t.kind === 'tasks') {
  return (
    <div
      key={t.id}
      role="button"
      tabIndex={0}
      onClick={() => activateTab(t.id)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') activateTab(t.id)
      }}
      title="Tasks"
      className={`${base} ${tone} cursor-pointer pl-3 pr-1.5`}
    >
      {accent}
      <ListTodo size={13} className="flex-none opacity-80" />
      <span className="truncate">Tasks</span>
      <button
        onClick={(e) => {
          e.stopPropagation()
          closeTab(t.id)
        }}
        aria-label="Close tab"
        title="Close"
        className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 ${
          active ? 'opacity-70' : 'opacity-0 group-hover:opacity-70'
        }`}
      >
        <X size={12} />
      </button>
    </div>
  )
}
```

## 8. Add host data only when needed

A static tab needs no bridge changes. If the tab reads or mutates daemon state,
add the full request/reply path instead of fabricating data in the component:

1. Add a `GuiReq` request type in `src-webgui/src/koma.d.ts`.
2. Handle the request in the Rust GUI host.
3. Add the Rust-to-JS push envelope type in `store/koma.ts`.
4. Add a reducer case in `useKoma.push`.
5. Store the authoritative result in the appropriate Zustand slice.
6. Add a store action such as `refreshTasks` that calls `req(...)`.
7. Request data from the tab or panel when it becomes active.

For a tab that must refresh whenever it is selected:

```tsx
const activeTabId = useKoma((s) => s.ui.activeTabId)
const refreshTasks = useKoma((s) => s.refreshTasks)

useEffect(() => {
  if (activeTabId === 'tasks') refreshTasks()
}, [activeTabId, refreshTasks])
```

Keep tokens, secrets, and other sensitive host data out of the push payload
unless the GUI needs to display them. Follow the existing optional-tolerant
handling for fields that may be absent while older host binaries are running.

## Verification checklist

After adding a menu and tab:

- `SidebarView` includes the new literal.
- `Sidebar.tsx` has the title, import, and panel branch.
- `ActivityBar.ITEMS` has the matching icon and label.
- The panel calls a store open-or-focus action.
- `Tab` and `KomaState` include the new tab and action.
- Repeated panel clicks focus one tab instead of duplicating it.
- `TabBar` renders and closes the tab.
- `TabbedMain` renders the tab and hides inactive content without unmounting it.
- Closing the active tab returns focus through the existing `closeTab` behavior.
- `cargo test` or the relevant GUI TypeScript/build command passes.

For architecture background, see [`ARCH_DESIGN_WEBGUI.md`](ARCH_DESIGN_WEBGUI.md).
