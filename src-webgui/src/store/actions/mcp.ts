import { normalizeGroups } from '../editorGroups'
import type { StoreGet, StoreSet } from '../api'
import type { KomaState } from '../state'
import type { Tab } from '../types/tabs'
import type { McpServer } from '../../types/config'

export const MCP_NEW_TAB = 'mcp:new'
export const MCP_IMPORT_TAB = 'mcp:import'
export const mcpEditTabId = (uuid: string) => `mcp:edit:${uuid}`

export function mcpTabTitle(name: string): string {
  return name.trim() || '(unnamed)'
}

/**
 * Follows the pushed server list: an edit tab whose server is gone is closed (a delete has
 * no ack on the wire, the next Config push is the signal) and a renamed server retitles its
 * tab. Returns the same object when nothing changed.
 */
export function reconcileMcpTabs(ui: KomaState['ui'], servers: McpServer[]): KomaState['ui'] {
  const byId = new Map(servers.map((s) => [s.id, s]))
  let changed = false
  const gone = new Set<string>()
  const tabs = ui.tabs.map((t) => {
    if (t.kind !== 'mcp' || t.mode !== 'edit') return t
    const srv = byId.get(t.uuid)
    if (!srv) {
      gone.add(t.id)
      return t
    }
    const title = mcpTabTitle(srv.name)
    if (title === t.title) return t
    changed = true
    return { ...t, title }
  })
  if (gone.size === 0) return changed ? { ...ui, tabs } : ui
  const remaining = tabs.filter((t) => !gone.has(t.id))
  return normalizeGroups({ ...ui, tabs: remaining, activeTabId: gone.has(ui.activeTabId) ? 'chat' : ui.activeTabId })
}

export function mcpTabActions(set: StoreSet, _get: StoreGet): Pick<KomaState, 'openMcpTab'> {
  return {
    openMcpTab: (target) => {
      const tab: Tab =
        target.mode === 'new'
          ? { id: MCP_NEW_TAB, kind: 'mcp', mode: 'new', title: 'New MCP server' }
          : target.mode === 'import'
            ? { id: MCP_IMPORT_TAB, kind: 'mcp', mode: 'import', title: 'Import MCP servers' }
            : { id: mcpEditTabId(target.uuid), kind: 'mcp', mode: 'edit', uuid: target.uuid, title: mcpTabTitle(target.title) }
      set((s) => {
        const base = normalizeGroups(s.ui)
        const tabs: Tab[] = base.tabs.some((t) => t.id === tab.id) ? base.tabs : [...base.tabs, tab]
        return { ui: normalizeGroups({ ...base, tabs, activeTabId: tab.id }) }
      })
    },
  }
}
