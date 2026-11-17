import { beforeEach, describe, expect, it } from 'vitest'
import { useKoma } from './koma'
import { fileKey } from './coding'

const root = '/w'

function setDir(entries: { name: string; path: string; isDir: boolean }[]) {
  useKoma.setState((state) => ({
    coding: {
      ...state.coding,
      activeRoot: root,
      dirs: { ...state.coding.dirs, [fileKey(root, '.koma')]: { entries, loading: false, error: null } },
    },
  }))
}

beforeEach(() => {
  useKoma.setState((state) => ({
    coding: { ...state.coding, activeRoot: null, dirs: {} },
    ui: {
      ...state.ui,
      tabs: [{ id: 'chat', kind: 'chat' }],
      activeTabId: 'chat',
      groups: ['g0'],
      tabGroup: { chat: 'g0' },
      groupActive: { g0: 'chat' },
      activeGroupId: 'g0',
      groupSizes: { g0: 1 },
      splitTree: { type: 'leaf', id: 'g0' },
    },
  }))
})

const welcomeTabs = () => useKoma.getState().ui.tabs.filter((t) => t.kind === 'welcome')

describe('openWelcomeTab', () => {
  it('adds one active tab and is idempotent', () => {
    useKoma.getState().openWelcomeTab('diagram')
    useKoma.getState().openWelcomeTab('diagram')
    const ui = useKoma.getState().ui
    expect(welcomeTabs()).toEqual([{ id: 'welcome-diagram', kind: 'welcome', which: 'diagram' }])
    expect(ui.activeTabId).toBe('welcome-diagram')
    expect(ui.tabGroup['welcome-diagram']).toBeDefined()
  })
  it('lands in the active group when the editor is split', () => {
    useKoma.setState((state) => ({
      ui: {
        ...state.ui,
        tabs: [...state.ui.tabs, { id: 'help', kind: 'help' }],
        groups: ['g0', 'g1'],
        splitTree: { type: 'split', id: 's0', dir: 'row', aSize: 1, bSize: 1, a: { type: 'leaf', id: 'g0' }, b: { type: 'leaf', id: 'g1' } },
        tabGroup: { chat: 'g0', help: 'g1' },
        activeGroupId: 'g1',
        activeTabId: 'help',
        groupActive: { g0: 'chat', g1: 'help' },
        groupSizes: { g0: 1, g1: 1 },
      },
    }))
    useKoma.getState().openWelcomeTab('design')
    expect(useKoma.getState().ui.tabGroup['welcome-design']).toBe('g1')
  })
})

describe('maybeOpenWelcomeTab', () => {
  it('opens with no workspace root and no listing', () => {
    useKoma.getState().maybeOpenWelcomeTab('design')
    expect(welcomeTabs()).toHaveLength(1)
    expect(useKoma.getState().ui.activeTabId).toBe('welcome-design')
  })
  it('opens even when files of the kind exist', () => {
    setDir([{ name: 'a.kdsgn', path: '.koma/a.kdsgn', isDir: false }])
    useKoma.getState().maybeOpenWelcomeTab('design')
    expect(welcomeTabs()).toHaveLength(1)
  })
  it('focuses the existing welcome tab instead of opening a second', () => {
    useKoma.getState().maybeOpenWelcomeTab('diagram')
    useKoma.getState().activateTab('chat')
    useKoma.getState().maybeOpenWelcomeTab('diagram')
    expect(welcomeTabs()).toHaveLength(1)
    expect(useKoma.getState().ui.activeTabId).toBe('welcome-diagram')
  })
  it('does not steal focus from an active editor tab of the same kind', () => {
    useKoma.setState((state) => ({
      ui: { ...state.ui, tabs: [...state.ui.tabs, { id: 'd1', kind: 'diagram' } as never], activeTabId: 'd1' },
    }))
    useKoma.getState().maybeOpenWelcomeTab('diagram')
    expect(welcomeTabs()).toHaveLength(0)
    expect(useKoma.getState().ui.activeTabId).toBe('d1')
  })
})
