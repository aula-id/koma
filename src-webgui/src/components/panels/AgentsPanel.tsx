import { useEffect, useMemo, useState } from 'react'
import { Plus } from 'lucide-react'
import { useKoma, resolveModelLabel, type AgentEntry, type CatalogueModelEntry, type CatalogueProviderEntry } from '../../store/koma'
import { Empty } from './helpers'
import { AccordionSection } from '../AccordionSection'

// A group's rows are hidden by default once they'd otherwise drown the flat
// list (an extension like Workflow can contribute 30+ sub-agents) — anything
// at or under this count starts expanded instead.
const GROUP_COLLAPSE_THRESHOLD = 5

// One agent row — shared by the flat (non-extension) list and each extension
// group's body so both render identically.
function AgentRow({
  a,
  catalogueModels,
  catalogueProviders,
  onClick,
}: {
  a: AgentEntry
  catalogueModels: CatalogueModelEntry[]
  catalogueProviders: CatalogueProviderEntry[]
  onClick: () => void
}) {
  return (
    <button
      onClick={onClick}
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
  )
}

// Sidebar Agents dashboard — clones McpPanel's layout language (master list +
// bottom "+ Add" button), but row click opens a full editor TAB (AgentTab, via
// the diff-tab-style per-agentId open-or-focus pattern) rather than an inline
// slide-in form: a prompt/tools editor wants real tab estate, not a narrow
// sidebar panel. The user's OWN agents (builtin/global/session) stay a flat
// list up top; agents contributed by an installed extension (`source ===
// 'extension'`) are bucketed underneath into one collapsible AccordionSection
// PER extension (same accordion the Connector panel uses for Providers/OAuth/
// Models) so a fleet of 30+ sub-agents from one extension doesn't drown the
// user's own roster. Fires GetAgents + a `store.installed` refresh once on
// mount (the latter resolves each group's header to the extension's display
// name rather than its raw id); the store holds the list thereafter, refreshed
// automatically by every SetAgent/DeleteAgent's own AgentsValues reply push
// (no polling, no manual re-request).
export function AgentsPanel() {
  const agents = useKoma((s) => s.agents)
  const catalogueModels = useKoma((s) => s.catalogueModels)
  const catalogueProviders = useKoma((s) => s.catalogueProviders)
  const installed = useKoma((s) => s.store.installed)
  const req = useKoma((s) => s.req)
  const openAgentTab = useKoma((s) => s.openAgentTab)
  const refreshInstalled = useKoma((s) => s.refreshInstalled)

  useEffect(() => {
    req({ r: 'GetAgents' })
    refreshInstalled()
  }, [req, refreshInstalled])

  // Split into the flat (non-extension) list + a per-extId group map, order
  // preserved from the host's list (insertion order into the Map).
  const { flatAgents, groups } = useMemo(() => {
    const flatAgents: AgentEntry[] = []
    const groups = new Map<string, AgentEntry[]>()
    for (const a of agents) {
      if (a.source === 'extension' && a.extId) {
        const list = groups.get(a.extId)
        if (list) list.push(a)
        else groups.set(a.extId, [a])
      } else {
        flatAgents.push(a)
      }
    }
    return { flatAgents, groups }
  }, [agents])

  // Expand state lives in component state only (no persistence) — seeded per
  // group the first time it's seen (default collapsed above the threshold,
  // else expanded) and left alone afterward so a manual toggle sticks across
  // AgentsValues re-pushes.
  const [openGroups, setOpenGroups] = useState<Record<string, boolean>>({})
  useEffect(() => {
    setOpenGroups((prev) => {
      let changed = false
      const next = { ...prev }
      for (const [extId, list] of groups) {
        if (!(extId in next)) {
          next[extId] = list.length <= GROUP_COLLAPSE_THRESHOLD
          changed = true
        }
      }
      return changed ? next : prev
    })
  }, [groups])

  return (
    <div className="absolute inset-0 flex min-h-0 flex-col overflow-hidden bg-koma-panel">
      <div className="flex-1 overflow-auto py-1">
        {agents.length === 0 && <Empty>No agents</Empty>}
        {flatAgents.map((a) => (
          <AgentRow
            key={a.name}
            a={a}
            catalogueModels={catalogueModels}
            catalogueProviders={catalogueProviders}
            onClick={() => openAgentTab(a.name)}
          />
        ))}
        {[...groups.entries()].map(([extId, list]) => {
          const label = installed.find((e) => e.id === extId)?.name || extId
          const open = openGroups[extId] ?? list.length <= GROUP_COLLAPSE_THRESHOLD
          return (
            <AccordionSection
              key={extId}
              title={`${label} (${list.length})`}
              open={open}
              onToggle={() => setOpenGroups((s) => ({ ...s, [extId]: !open }))}
            >
              {list.map((a) => (
                <AgentRow
                  key={a.name}
                  a={a}
                  catalogueModels={catalogueModels}
                  catalogueProviders={catalogueProviders}
                  onClick={() => openAgentTab(a.name)}
                />
              ))}
            </AccordionSection>
          )
        })}
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
