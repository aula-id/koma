import { useState, useEffect, useMemo } from 'react'
import { Plus, Trash2, Edit3, Link2 } from 'lucide-react'
import { useKoma } from '../../store/koma'

type RemotePanelView =
  | { kind: 'list' }
  | { kind: 'form'; draft: RemoteHostDraft; isNew: boolean }

type RemoteHostDraft = {
  name: string
  user: string
  host: string
  port: number
  keyPath: string
}

const emptyDraft = (): RemoteHostDraft => ({
  name: '',
  user: 'root',
  host: '',
  port: 22,
  keyPath: '',
})

export function RemotePanel() {
  const remoteHosts = useKoma((s) => s.remoteHosts)
  const push = useKoma((s) => s.req)
  const [view, setView] = useState<RemotePanelView>({ kind: 'list' })
  const [query, setQuery] = useState('')
  const [editId, setEditId] = useState<string | null>(null)

  useEffect(() => {
    push({ r: 'GetRemoteHosts' })
  }, [push])

  const filtered = useMemo(() => {
    if (!query) return remoteHosts
    const q = query.toLowerCase()
    return remoteHosts.filter(
      (h) =>
        h.name.toLowerCase().includes(q) ||
        h.host.toLowerCase().includes(q) ||
        h.user.toLowerCase().includes(q),
    )
  }, [remoteHosts, query])

  const handleAdd = () => {
    setEditId(null)
    setView({ kind: 'form', draft: emptyDraft(), isNew: true })
  }
  const handleEdit = (host: RemoteHost) => {
    setEditId(host.id)
    setView({
      kind: 'form',
      draft: {
        name: host.name,
        user: host.user,
        host: host.host,
        port: host.port,
        keyPath: host.keyPath ?? '',
      },
      isNew: false,
    })
  }
  const handleConnect = (hostId: string) => push({ r: 'ConnectRemoteHost', hostId })
  const handleDelete = (hostId: string) => push({ r: 'DeleteRemoteHost', id: hostId })

  const formatLastSeen = (ts: number | null) => {
    if (!ts) return 'Never connected'
    const diff = Math.floor(Date.now() / 1000) - ts
    if (diff < 60) return 'Connected just now'
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`
    return `${Math.floor(diff / 86400)}d ago`
  }

  const handleSave = () => {
    if (view.kind !== 'form') return
    const { draft, isNew } = view
    if (!draft.name || !draft.host) return
    if (isNew) {
      push({
        r: 'AddRemoteHost',
        name: draft.name,
        user: draft.user,
        host: draft.host,
        port: draft.port,
        keyPath: draft.keyPath || null,
      })
    } else if (editId) {
      push({
        r: 'EditRemoteHost',
        id: editId,
        name: draft.name,
        user: draft.user,
        host: draft.host,
        port: draft.port,
        keyPath: draft.keyPath || null,
      })
    }
    setEditId(null)
    setView({ kind: 'list' })
  }

  if (view.kind === 'form') {
    const { draft, isNew } = view
    return (
      <div className="flex flex-col h-full overflow-hidden text-xs">
        <div className="flex-none flex items-center justify-between p-2 pb-1">
          <span className="text-koma-fg font-medium">
            {isNew ? 'Add Remote Host' : 'Edit Remote Host'}
          </span>
          <button
            className="text-koma-dim hover:text-koma-fg"
            onClick={() => { setEditId(null); setView({ kind: 'list' }) }}
          >
            Cancel
          </button>
        </div>
        <div className="flex-1 overflow-auto px-2 py-1 flex flex-col gap-1.5">
          <label className="text-koma-dim">Name</label>
          <input
            className="bg-koma-panel border border-koma-border rounded px-2 py-1 text-koma-fg"
            value={draft.name}
            onChange={(e) => setView({ kind: 'form', draft: { ...draft, name: e.target.value }, isNew })}
            placeholder="prod-server"
          />
          <label className="text-koma-dim">User</label>
          <input
            className="bg-koma-panel border border-koma-border rounded px-2 py-1 text-koma-fg"
            value={draft.user}
            onChange={(e) => setView({ kind: 'form', draft: { ...draft, user: e.target.value }, isNew })}
            placeholder="root"
          />
          <label className="text-koma-dim">Host</label>
          <input
            className="bg-koma-panel border border-koma-border rounded px-2 py-1 text-koma-fg"
            value={draft.host}
            onChange={(e) => setView({ kind: 'form', draft: { ...draft, host: e.target.value }, isNew })}
            placeholder="192.168.1.10"
          />
          <label className="text-koma-dim">Port</label>
          <input
            type="number"
            className="bg-koma-panel border border-koma-border rounded px-2 py-1 text-koma-fg"
            value={draft.port}
            onChange={(e) => setView({ kind: 'form', draft: { ...draft, port: Number(e.target.value) || 22 }, isNew })}
          />
          <label className="text-koma-dim">Key Path (optional)</label>
          <input
            className="bg-koma-panel border border-koma-border rounded px-2 py-1 text-koma-fg"
            value={draft.keyPath}
            onChange={(e) => setView({ kind: 'form', draft: { ...draft, keyPath: e.target.value }, isNew })}
            placeholder="~/.ssh/id_ed25519"
          />
        </div>
        <div className="flex-none border-t border-koma-border p-2">
          <button
            className="flex w-full items-center justify-center gap-1.5 rounded bg-koma-accent text-koma-bg py-1.5 text-[12px] hover:opacity-90"
            onClick={handleSave}
          >
            {isNew ? 'Add Host' : 'Save'}
          </button>
        </div>
      </div>
    )
  }

  return (
    <div className="flex flex-col h-full overflow-hidden text-xs">
      {/* Search */}
      <div className="flex-none p-2 pb-1">
        <input
          className="w-full bg-koma-panel border border-koma-border rounded px-2 py-1 text-koma-fg placeholder-koma-dim"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search hosts..."
        />
      </div>

      {/* Host list — scrollable, fills remaining space */}
      <div className="flex-1 overflow-auto px-2 py-1">
        <div className="flex flex-col gap-1">
          {filtered.length === 0 && (
            <div className="text-koma-dim text-center py-4">No hosts saved</div>
          )}
          {filtered.map((host) => (
            <div
              key={host.id}
              className="flex items-center justify-between bg-koma-panel border border-koma-border rounded px-2 py-1.5 hover:bg-koma-hover"
            >
              <div className="flex items-center gap-2 min-w-0">
                <span
                  className={`inline-block h-2.5 w-2.5 flex-none rounded-full ${
                    host.connected
                      ? 'bg-koma-success'
                      : host.lastConnected
                        ? 'bg-koma-warn'
                        : 'bg-koma-dim'
                  }`}
                  title={formatLastSeen(host.lastConnected)}
                />
                <div className="min-w-0">
                  <div className="flex items-center gap-1.5">
                    <span className="text-koma-fg font-medium truncate">{host.name}</span>
                  </div>
                  <div className="text-koma-dim truncate">
                    {host.user}@{host.host}:{host.port}
                  </div>
                </div>
              </div>
              <div className="flex items-center gap-1">
                <button
                  className="p-1 text-koma-accent hover:text-koma-fg"
                  title="Connect"
                  onClick={() => handleConnect(host.id)}
                >
                  <Link2 size={14} />
                </button>
                <button
                  className="p-1 text-koma-dim hover:text-koma-fg"
                  title="Edit"
                  onClick={() => handleEdit(host)}
                >
                  <Edit3 size={14} />
                </button>
                <button
                  className="p-1 text-koma-dim hover:text-koma-error"
                  title="Delete"
                  onClick={() => handleDelete(host.id)}
                >
                  <Trash2 size={14} />
                </button>
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Add button — pinned to bottom */}
      <div className="flex-none border-t border-koma-border p-2">
        <button
          className="flex w-full items-center justify-center gap-1.5 rounded border border-koma-border py-1.5 text-[12px] text-koma-accent hover:bg-koma-hover"
          onClick={handleAdd}
        >
          <Plus size={14} />
          Add Host
        </button>
      </div>
    </div>
  )
}
