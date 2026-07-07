import { Plus, Pencil, Trash2, Check, X } from 'lucide-react'

type Transport = 'stdio' | 'http'

type Server = {
  id: string
  name: string
  enabled: boolean
  transport: Transport
  command: string
  args: string
  env: string
  url: string
}

type Props = {
  servers: Server[]
  armed: string | null
  onAdd: () => void
  onEdit: (s: Server) => void
  onArm: (id: string) => void
  onDisarm: () => void
  onConfirm: (id: string) => void
}

export function McpListView({ servers, armed, onAdd, onEdit, onArm, onDisarm, onConfirm }: Props) {
  return (
    <>
      <div className="flex-1 overflow-auto py-1">
        {servers.length === 0 && (
          <div className="px-3 py-6 text-center text-[12px] text-koma-fg opacity-35">
            No MCP servers
          </div>
        )}
        {servers.map((s) => (
          <div
            key={s.id}
            className="group flex min-h-[38px] items-center gap-2.5 px-3 py-1.5 hover:bg-koma-hover"
          >
            {armed === s.id ? (
              <div className="flex flex-1 items-center gap-2">
                <span className="flex-1 truncate text-[12px] text-koma-fg">
                  Delete "{s.name || 'server'}"?
                </span>
                <button
                  onClick={() => onConfirm(s.id)}
                  aria-label="Confirm delete"
                  className="flex h-5 w-5 items-center justify-center rounded text-koma-fg opacity-70 transition-colors hover:text-emerald-500 hover:opacity-100"
                >
                  <Check size={14} />
                </button>
                <button
                  onClick={onDisarm}
                  aria-label="Cancel delete"
                  className="flex h-5 w-5 items-center justify-center rounded text-koma-fg opacity-70 transition-colors hover:text-red-500 hover:opacity-100"
                >
                  <X size={14} />
                </button>
              </div>
            ) : (
              <>
                <span
                  className={`h-2 w-2 flex-none rounded-full ${s.enabled ? 'bg-emerald-500' : 'bg-koma-fg/25'}`}
                />
                <button onClick={() => onEdit(s)} className="min-w-0 flex-1 text-left">
                  <div className="flex items-center gap-2">
                    <span
                      className={`truncate text-[13px] text-koma-fg ${s.enabled ? '' : 'opacity-50'}`}
                    >
                      {s.name || '(unnamed)'}
                    </span>
                    <span className="flex-none rounded bg-koma-head px-1 py-px text-[10px] uppercase tracking-wide text-koma-fg opacity-60">
                      {s.transport}
                    </span>
                  </div>
                </button>
                <button
                  onClick={() => onEdit(s)}
                  aria-label="Edit"
                  className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-0 transition group-hover:opacity-60 hover:!opacity-100"
                >
                  <Pencil size={13} />
                </button>
                <button
                  onClick={() => onArm(s.id)}
                  aria-label="Delete"
                  className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-0 transition group-hover:opacity-60 hover:!text-red-500 hover:!opacity-100"
                >
                  <Trash2 size={13} />
                </button>
              </>
            )}
          </div>
        ))}
      </div>
      <div className="flex-none border-t border-koma-border p-2">
        <button
          onClick={onAdd}
          className="flex w-full items-center justify-center gap-1.5 rounded border border-koma-border py-1.5 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
        >
          <Plus size={14} /> Add server
        </button>
      </div>
    </>
  )
}
