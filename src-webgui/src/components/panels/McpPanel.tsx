import { Plus } from 'lucide-react'

// Design-phase stub. No server data yet — empty state + an inert add affordance.
export function McpPanel() {
  return (
    <div className="flex flex-col py-2">
      <div className="px-4 py-6 text-center text-[12px] text-koma-fg opacity-35">No MCP servers</div>
      <button className="mx-3 flex items-center justify-center gap-1.5 rounded border border-koma-border py-1.5 text-[12px] text-koma-fg opacity-70 transition hover:bg-koma-hover hover:opacity-100">
        <Plus size={14} /> Add server
      </button>
    </div>
  )
}
