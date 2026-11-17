import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useKoma } from './koma'
import { fileKey } from './coding'

const root = '/w'

function setDir(entries: { name: string; path: string; isDir: boolean }[], extra: { loading?: boolean; error?: string | null } = {}) {
  useKoma.setState((state) => ({
    coding: {
      ...state.coding,
      activeRoot: root,
      dirs: { ...state.coding.dirs, [fileKey(root, '.koma')]: { entries, loading: false, error: null, ...extra } },
    },
  }))
}

beforeEach(() => {
  vi.useFakeTimers()
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
afterEach(() => vi.useRealTimers())

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
  it('opens nothing when files of the kind exist', () => {
    setDir([{ name: 'a.kdsgn', path: '.koma/a.kdsgn', isDir: false }])
    useKoma.getState().maybeOpenWelcomeTab('design')
    expect(welcomeTabs()).toHaveLength(0)
  })
  it('opens one tab when the listing is known and empty', () => {
    setDir([])
    useKoma.getState().maybeOpenWelcomeTab('design')
    expect(welcomeTabs()).toHaveLength(1)
  })
  it('treats a missing .koma directory as no files', () => {
    setDir([], { error: 'No such file or directory (os error 2)' })
    useKoma.getState().maybeOpenWelcomeTab('diagram')
    expect(welcomeTabs()).toHaveLength(1)
  })
  it('does not open on an unrelated listing error', () => {
    setDir([], { error: 'permission denied' })
    useKoma.getState().maybeOpenWelcomeTab('diagram')
    expect(welcomeTabs()).toHaveLength(0)
  })
  it('waits for an unknown listing instead of treating it as empty', () => {
    const refresh = vi.fn()
    useKoma.setState((state) => ({ refreshCodingDir: refresh, coding: { ...state.coding, activeRoot: root, dirs: {} } }))
    useKoma.getState().maybeOpenWelcomeTab('diagram')
    expect(refresh).toHaveBeenCalledWith(root, '.koma')
    expect(welcomeTabs()).toHaveLength(0)
    setDir([{ name: 'a.diag', path: '.koma/a.diag', isDir: false }])
    vi.advanceTimersByTime(200)
    expect(welcomeTabs()).toHaveLength(0)
  })
})
