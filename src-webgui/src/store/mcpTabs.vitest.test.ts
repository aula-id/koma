import { beforeEach, describe, expect, it } from 'vitest'
import { useKoma } from './koma'
import { reconcileMcpTabs } from './actions/mcp'
import type { McpServer } from '../types/config'

const srv = (id: string, name: string): McpServer =>
  ({ id, name, enabled: true, transport: 'stdio', command: '', args: '', env: '', url: '', toolCount: 0 }) as McpServer

beforeEach(() => {
  useKoma.setState((state) => ({
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

const mcpTabs = () => useKoma.getState().ui.tabs.filter((t) => t.kind === 'mcp')

describe('openMcpTab', () => {
  it('opens one new tab and one import tab, focusing instead of duplicating', () => {
    const { openMcpTab } = useKoma.getState()
    openMcpTab({ mode: 'new' })
    openMcpTab({ mode: 'import' })
    openMcpTab({ mode: 'new' })
    expect(mcpTabs().map((t) => t.id)).toEqual(['mcp:new', 'mcp:import'])
    expect(useKoma.getState().ui.activeTabId).toBe('mcp:new')
    expect(mcpTabs()[0]).toMatchObject({ title: 'New MCP server' })
    expect(mcpTabs()[1]).toMatchObject({ title: 'Import MCP servers' })
  })

  it('keeps one edit tab per server uuid and focuses the existing one', () => {
    const { openMcpTab } = useKoma.getState()
    openMcpTab({ mode: 'edit', uuid: 'a', title: 'alpha' })
    openMcpTab({ mode: 'edit', uuid: 'b', title: 'beta' })
    openMcpTab({ mode: 'edit', uuid: 'a', title: 'alpha' })
    expect(mcpTabs()).toHaveLength(2)
    expect(useKoma.getState().ui.activeTabId).toBe('mcp:edit:a')
    expect(mcpTabs()[0]).toMatchObject({ mode: 'edit', uuid: 'a', title: 'alpha' })
  })
})

describe('Config push reconciliation', () => {
  it('closes the tab of a server that disappeared, retitles renamed ones', () => {
    const { openMcpTab } = useKoma.getState()
    openMcpTab({ mode: 'edit', uuid: 'a', title: 'alpha' })
    openMcpTab({ mode: 'edit', uuid: 'b', title: 'beta' })
    openMcpTab({ mode: 'new' })
    const next = reconcileMcpTabs(useKoma.getState().ui, [srv('b', 'beta2')])
    expect(next.tabs.filter((t) => t.kind === 'mcp').map((t) => [t.id, t.title])).toEqual([
      ['mcp:edit:b', 'beta2'],
      ['mcp:new', 'New MCP server'],
    ])
    expect(next.activeTabId).toBe('mcp:new')
  })

  it('falls back to chat when the active edit tab disappears, and leaves new/import alone', () => {
    const { openMcpTab } = useKoma.getState()
    openMcpTab({ mode: 'import' })
    openMcpTab({ mode: 'edit', uuid: 'a', title: 'alpha' })
    const next = reconcileMcpTabs(useKoma.getState().ui, [])
    expect(next.activeTabId).toBe('chat')
    expect(next.tabs.map((t) => t.id)).toEqual(['chat', 'mcp:import'])
  })

  it('returns the same ui when nothing changed', () => {
    useKoma.getState().openMcpTab({ mode: 'edit', uuid: 'a', title: 'alpha' })
    const ui = useKoma.getState().ui
    expect(reconcileMcpTabs(ui, [srv('a', 'alpha')])).toBe(ui)
  })
})
