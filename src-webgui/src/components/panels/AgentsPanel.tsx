import { useEffect } from 'react'
import { Plus } from 'lucide-react'
import { useKoma, resolveModelLabel } from '../../store/koma'
import { Empty } from './helpers'

// Sidebar Agents dashboard — clones McpPanel's layout language (master list +
// bottom "+ Add" button), but row click opens a full editor TAB (AgentTab, via
// the diff-tab-style per-agentId open-or-focus pattern) rather than an inline
// slide-in form: a prompt/tools editor wants real tab estate, not a narrow
// sidebar panel. One flat list (no accordion — unlike Connector, there's only
// one catalogue here). Fires GetAgents once on mount; the store holds the
// list thereafter, refreshed automatically by every SetAgent/DeleteAgent's own
// AgentsValues reply push (no polling, no manual re-request).
export function AgentsPanel() {
  const agents = useKoma((s) => s.agents)
  const catalogueModels = useKoma((s) => s.catalogueModels)
  const catalogueProviders = useKoma((s) => s.catalogueProviders)
  const req = useKoma((s) => s.req)
  const openAgentTab = useKoma((s) => s.openAgentTab)

  useEffect(() => {
    req({ r: 'GetAgents' })
  }, [req])

  return (
    <div className="absolute inset-0 flex min-h-0 flex-col overflow-hidden bg-koma-panel">
      <div className="flex-1 overflow-auto py-1">
        {agents.length === 0 && <Empty>No agents</Empty>}
        {agents.map((a) => (
          <button
            key={a.name}
            onClick={() => openAgentTab(a.name)}
            className="group flex min-h-[42px] w-full items-center gap-2 px-3 py-1.5 text-left hover:bg-koma-hover"
          >
            {/* Fixed-width glyph column (TUI settings parity: `"* "` dim for
                global, two spaces for local) — reserved even when blank so
                names stay column-aligned regardless of scope. */}
            <span
              aria-hidden
              className="w-3 flex-none text-center text-[12px] text-koma-fg opacity-50"
            >
              {a.source === 'global' ? '*' : ''}
            </span>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-1.5">
                <span className="truncate text-[13px] text-koma-fg">{a.name}</span>
                {a.source === 'builtin' && (
                  <span className="flex-none rounded bg-koma-head px-1 py-px text-[10px] uppercase tracking-wide text-koma-fg opacity-60">
                    built-in
                  </span>
                )}
              </div>
              <div className="truncate text-[11px] text-koma-fg opacity-45">
                {resolveModelLabel(a.modelUuid, catalogueModels, catalogueProviders)}
              </div>
            </div>
          </button>
        ))}
      </div>
      <div className="flex-none border-t border-koma-border p-2">
        <button
          onClick={() => openAgentTab(null)}
          className="flex w-full items-center justify-center gap-1.5 rounded border border-koma-border py-1.5 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
        >
          <Plus size={14} /> Add agent
        </button>
      </div>
    </div>
  )
}
