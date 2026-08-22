import { useState, useEffect, useMemo, useRef, type FormEvent, type KeyboardEvent } from 'react'
import { Plus, Trash2, Edit3, Link2, Check, X, Lock } from 'lucide-react'
import { useKoma } from '../../store/koma'
import { BrailleSpinner } from '../BrailleSpinner'

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

/** States where the connection is in-progress (not idle, not ready hub). */
const ACTIVE_STATES = ['resolving', 'auth_required', 'bootstrapping', 'connecting']
/** Host is live (ctx retained) — green dot / disconnect available. */
const LIVE_STATES = ['ready', 'connecting', 'connected']

export function RemotePanel() {
  const remoteHosts = useKoma((s) => s.remoteHosts)
  const remoteState = useKoma((s) => s.remoteState)
  const push = useKoma((s) => s.req)
  const [view, setView] = useState<RemotePanelView>({ kind: 'list' })
  const [query, setQuery] = useState('')
  const [editId, setEditId] = useState<string | null>(null)
  const [armedDelete, setArmedDelete] = useState<string | null>(null)

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
  const isBusy = ACTIVE_STATES.includes(remoteState.state)
  const isHostLive = (hostId: string) =>
    remoteState.hostId === hostId && LIVE_STATES.includes(remoteState.state)
  const handleConnect = (hostId: string) => {
    if (isBusy || remoteState.state === 'ready' || remoteState.state === 'connected') return
    push({ r: 'ConnectRemoteHost', hostId })
  }
  const handleDisconnect = () => {
    push({ r: 'DisconnectRemoteHost', hostId: remoteState.hostId ?? '' })
  }
  const confirmDelete = (hostId: string) => {
    push({ r: 'DeleteRemoteHost', id: hostId })
    setArmedDelete(null)
  }

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
          className="w-full bg-koma-panel border border-koma-border rounded px-2 py-1 text-koma-fg placeholder-koma-dim outline-none focus:border-koma-accent"
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
          {filtered.map((host) => {
            const armed = armedDelete === host.id
            const isActiveHost = isBusy && remoteState.hostId === host.id
            const isOtherBusy = isBusy && remoteState.hostId !== host.id

            /* ── Active host: inline loading / auth state ── */
            if (isActiveHost) {
              return (
                <div
                  key={host.id}
                  className="flex min-h-[49px] items-center justify-between bg-koma-panel border border-koma-accent/40 rounded px-2 py-1.5"
                >
                  <div className="flex items-center gap-2 min-w-0">
                    <BrailleSpinner size={13} className="text-koma-accent" />
                    <div className="min-w-0">
                      <div className="text-koma-fg font-medium truncate">{host.name}</div>
                      <div className="text-koma-dim truncate">
                        {remoteState.state === 'auth_required'
                          ? 'Waiting for password…'
                          : `${remoteState.state.replace('_', ' ')}…`}
                      </div>
                    </div>
                  </div>
                  <button
                    className="flex-none rounded border border-koma-border px-2 py-0.5 text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
                    onClick={() => push({ r: 'CancelRemoteConnect' })}
                  >
                    Cancel
                  </button>
                </div>
              )
            }

            /* ── Normal / armed / disabled host row ── */
            return (
            <div
              key={host.id}
              className={`flex min-h-[49px] items-center justify-between bg-koma-panel border border-koma-border rounded px-2 py-1.5 ${isOtherBusy ? 'opacity-40 pointer-events-none' : 'hover:bg-koma-hover'}`}
            >
              {armed ? (
                <>
                  <span className="min-w-0 flex-1 truncate text-[12px] text-koma-error">
                    Delete {host.name} forever?
                  </span>
                  <div className="flex flex-none items-center gap-1">
                    <button
                      autoFocus
                      aria-label={`Confirm delete ${host.name}`}
                      title="Confirm delete"
                      className="flex items-center gap-1 rounded px-2 py-1 text-koma-error hover:bg-koma-error/10"
                      onClick={() => confirmDelete(host.id)}
                    >
                      <Check size={13} />
                      yes
                    </button>
                    <button
                      aria-label="Cancel delete"
                      title="Cancel"
                      className="flex items-center gap-1 rounded px-2 py-1 text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
                      onClick={() => setArmedDelete(null)}
                    >
                      <X size={13} />
                      no
                    </button>
                  </div>
                </>
              ) : (
                <>
              <div className="flex items-center gap-2 min-w-0">
                <span
                  className={`inline-block h-2.5 w-2.5 flex-none rounded-full ${
                    isHostLive(host.id) || host.connected
                      ? 'bg-koma-success'
                      : host.lastConnected
                        ? 'bg-koma-warn'
                        : 'bg-koma-dim'
                  }`}
                  title={
                    isHostLive(host.id)
                      ? remoteState.state === 'ready'
                        ? 'Host ready'
                        : remoteState.state === 'connected'
                          ? 'Session attached'
                          : 'Connecting…'
                      : formatLastSeen(host.lastConnected)
                  }
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
                {isHostLive(host.id) ? (
                  <button
                    className="rounded border border-koma-border px-2 py-0.5 text-[11px] text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100"
                    title="Disconnect"
                    onClick={handleDisconnect}
                  >
                    Disconnect
                  </button>
                ) : (
                  <button
                    disabled={isBusy || remoteState.state === 'ready' || remoteState.state === 'connected'}
                    className="p-1 text-koma-accent hover:text-koma-fg disabled:cursor-not-allowed disabled:opacity-40"
                    title="Connect"
                    onClick={() => handleConnect(host.id)}
                  >
                    <Link2 size={14} />
                  </button>
                )}
                <button
                  disabled={isBusy}
                  className="p-1 text-koma-dim hover:text-koma-fg disabled:cursor-not-allowed disabled:opacity-40"
                  title="Edit"
                  onClick={() => handleEdit(host)}
                >
                  <Edit3 size={14} />
                </button>
                <button
                  disabled={isBusy}
                  className="p-1 text-koma-dim hover:text-koma-error disabled:cursor-not-allowed disabled:opacity-40"
                  title="Delete"
                  onClick={() => setArmedDelete(host.id)}
                >
                  <Trash2 size={14} />
                </button>
              </div>
                </>
              )}
            </div>
            )
          })}
        </div>
      </div>

      {/* Add button — pinned to bottom */}
      <div className="flex-none border-t border-koma-border p-2">
        <button
          disabled={isBusy}
          className="flex w-full items-center justify-center gap-1.5 rounded border border-koma-border py-1.5 text-[12px] text-koma-accent hover:bg-koma-hover disabled:cursor-not-allowed disabled:opacity-40"
          onClick={handleAdd}
        >
          <Plus size={14} />
          Add Host
        </button>
      </div>
    </div>
  )
}
