import { useEffect, useRef, useState } from 'react'
import { FolderOpen } from 'lucide-react'
import { useKoma } from '../store/koma'

let sequence = 0
const pending = new Map<string, (path: string | null) => void>()

function pickFolder(): Promise<string | null> {
  return new Promise((resolve, reject) => {
    const requestId = `settings-folder-${++sequence}`
    pending.set(requestId, resolve)
    window.__komaFolderReply = (reply) => {
      const callback = pending.get(reply.requestId)
      pending.delete(reply.requestId)
      callback?.(reply.path)
    }
    try {
      window.ipc!.postMessage(JSON.stringify({ t: 'req', r: 'PickSettingsFolder', requestId }))
    } catch (error) {
      pending.delete(requestId)
      reject(error)
    }
  })
}

export function SettingsFolderButton({ label, onPick, compact = false }: {
  label: string; onPick: (path: string) => void; compact?: boolean
}) {
  const local = useKoma((s) => s.remoteState.hostId === null && s.remoteState.state === 'disconnected')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState(false)
  const callback = useRef(onPick)
  callback.current = onPick
  const mounted = useRef(true)
  useEffect(() => {
    mounted.current = true
    return () => { mounted.current = false }
  }, [])
  if (!local || !window.ipc) return null
  const choose = async () => {
    if (busy) return
    const sessionId = useKoma.getState().session.id
    setBusy(true)
    setError(false)
    try {
      const path = await pickFolder()
      const state = useKoma.getState()
      if (mounted.current && path && state.session.id === sessionId && state.remoteState.hostId === null && state.remoteState.state === 'disconnected') {
        callback.current(path)
      }
    } catch { if (mounted.current) setError(true) }
    finally { if (mounted.current) setBusy(false) }
  }
  return <span className="inline-flex flex-col items-start gap-1">
    <button type="button" onClick={() => { void choose() }} disabled={busy} aria-label={label} title={label}
      className={`flex h-8 flex-none items-center justify-center gap-1 rounded border border-koma-border text-koma-fg hover:bg-koma-hover disabled:cursor-not-allowed disabled:opacity-40 ${compact ? 'w-8' : 'px-2 text-[11px]'}`}>
      <FolderOpen size={13} />{compact ? null : label}
    </button>
    {error && <span role="alert" className="text-[11px] text-koma-error">Could not open folder picker.</span>}
  </span>
}
