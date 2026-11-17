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
| Tab type | `src-webgui/src/store/types/tabs.ts` |
| Store state interface (`KomaState`) | `src-webgui/src/store/state.ts` |
| Tab open/close actions | `src-webgui/src/store/actions/*.ts` (`session.ts`, `tabs.ts`, `welcome.ts`, ...) |
| Host push reducers | `src-webgui/src/store/push/*.ts` |
| Tab strip rendering | `src-webgui/src/components/TabBar.tsx` |
| Main tab content routing (`TabBody`) | `src-webgui/src/routes/index.tsx` |
| New tab content | `src-webgui/src/components/<Name>Tab.tsx` |
| Rust bridge request types, if needed | `src-webgui/src/koma.d.ts` (`GuiReq`) and the Rust GUI host |

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
  | 'skills'
  | 'usage'
  | 'store'
  | 'remote'
  | 'diagram'
  | 'design'

const TITLES: Record<SidebarView, string> = {
  explore: 'Chat',
  coding: 'Coding',
  git: 'Source Control',
  mcp: 'MCP Servers',
  connector: 'Connector',
  importGraph: 'Import Graph',
  agents: 'Agents',
  skills: 'Skills',
  usage: 'Usage',
  store: 'Extensions',
  remote: 'Remote',
  diagram: 'Diagram',
  design: 'Design',
}
```

Add the panel beside the other panel branches:

```tsx
{view === 'tasks' && <TasksPanel />}
```

(You must also extend `SidebarView` / `TITLES` with your new literal. The example
above shows the built-in set; a new view is an additive union member. The first
view keeps the id `explore` but is titled and labelled "Chat".)

`RootLayout` (in `routes/index.tsx`) already stores `activeView` and handles the standard behavior:
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
that does not have a matching Sidebar view. The same list drives the Settings
"Sidebar" section (order and hide toggles); items that are hidden or do not fit
the bar height move into the "..." overflow menu.

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

For empty states, reuse the primitives in `panels/helpers.tsx`: `Empty` for a
single empty section of a populated panel, `EmptyState` for a panel with
nothing in it (at most one primary and one secondary action), `NoProjectState`
and `NoWorkspaceState` when the feature needs a project folder, and `AddBtn`
(with a `label`, and optional visible `text`) for header actions.

## 4. Add the tab to the Zustand store

For a singleton tab, add a fixed ID and a new `kind` to the `Tab` union in
`src-webgui/src/store/types/tabs.ts`:

```tsx
export type Tab =
  | { id: 'chat'; kind: 'chat' }
  | { id: 'tasks'; kind: 'tasks' }
  // existing variants...
```

Add the action to `KomaState` in `src-webgui/src/store/state.ts`:

```tsx
openTasksTab: () => void
```

Implement it beside `openSettingsTab` and `openHelpTab` in
`store/actions/session.ts` (add the name to that file's `Pick<KomaState, ...>`
return type), or in the actions file for your feature:

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
notifications
graph
import-graph
analytics
store
upload-skill
tasks
welcome-diagram / welcome-design
```

Use a stable, data-derived ID when users can open multiple instances, such as
file diffs:

```text
diff:<path>
gitdiff:<staged|unstaged>:<path>
commitdiff:<sha>:<path>
sa:<agentId>
bash:<jobId>
```

If a tab's identity can change after editing, keep its React identity stable
and update its data key separately. The agent editor tabs (`kind: 'agent'`)
follow this pattern.

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
for scrollable tab content so it fits inside the tab area.

## 6. Render the tab from `TabBody`

Import the tab in `src-webgui/src/routes/index.tsx`:

```tsx
import { TasksTab } from '../components/TasksTab'
```

Add a branch to the `kind` chain in `TabBody`:

```tsx
) : tab.kind === 'tasks' ? (
  <TasksTab />
) : null}
```

`TabbedMain` wraps every non-chat tab in a container and gives inactive tabs the
`hidden` class instead of unmounting them, so local component state survives tab
switches. The same container handles editor groups and split layouts, so a tab
body does not position itself. Close the tab through the existing `closeTab`
action; do not add a second close implementation in the tab content.

## 7. Add the tab strip entry

`TabBar.tsx` renders every tab with one shared element, so a new tab does not
need its own JSX branch. Add a `case` to `tabVisual` that returns the icon,
label and title:

```tsx
case 'tasks':
  return { Icon: ListTodo, label: 'Tasks', title: 'Tasks' }
```

The shared element already provides click-to-activate, Enter and Space handling,
drag reorder, the context menu, the active accent, and a close button that calls
`requestClose`. `requestClose` routes to `closeTab` and adds the dirty-close
confirmation for git tool, coding file, diagram and design tabs. Only extend
`requestClose` if your tab can hold unsaved state.

## 8. Add host data only when needed

A static tab needs no bridge changes. If the tab reads or mutates daemon state,
add the full request/reply path instead of fabricating data in the component:

1. Add a `GuiReq` request type in `src-webgui/src/koma.d.ts`.
2. Handle the request in the Rust GUI host.
3. Add the Rust-to-JS push envelope type next to the other envelopes in `store/`.
4. Add a reducer case in the matching `store/push/*.ts` module (`useKoma.push`
   in `store/koma.ts` tries each `pushXxx` reducer in turn).
5. Store the authoritative result in the appropriate Zustand slice.
6. Add a store action such as `refreshTasks` that calls `req(...)` with the new
   `GuiReq`.
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
- `ACTIVITY_BAR_ITEMS` has the matching icon and label.
- The panel calls a store open-or-focus action.
- Repeated panel clicks focus one tab instead of duplicating it.
- `Tab` and `KomaState` include the new tab and action.
- `TabBar` (`tabVisual`) renders and closes the tab.
- `TabBody` renders the tab and `TabbedMain` hides inactive content without unmounting it.
- Closing the active tab returns focus through the existing `closeTab` behavior.
- `npm test` (`vite build && vitest run`) in `src-webgui` passes; `npm run test:logic`
  runs the non-browser project only.

For architecture background, see [`ARCH_DESIGN_WEBGUI.md`](ARCH_DESIGN_WEBGUI.md).
