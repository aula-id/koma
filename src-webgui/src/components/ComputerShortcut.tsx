import { LoaderCircle, Monitor } from 'lucide-react'
import { useKoma } from '../store/koma'
import { useComputerPreview } from '../store/computerPreview'

/** Native titlebar shortcut; detailed controls remain in Settings. */
export function ComputerShortcut() {
  const session = useKoma(s => s.session.id)
  const remote = useKoma(s => s.remoteState.state)
  const computer = useKoma(s => s.computer)
  const req = useKoma(s => s.req)
  const pending = useComputerPreview(s => s.requestedSession === session && !!session)
  const enabled = !!computer?.enabled && computer.session === session
  if (!session || ['ready', 'connected', 'connecting'].includes(remote)) return null
  const label = pending ? 'Enabling computer use…' : enabled ? 'Stop computer use' : 'Enable computer use'
  return <button type="button" aria-label={label} title={label} aria-pressed={enabled} disabled={pending}
    className={`pointer-events-auto flex h-[22px] flex-none items-center rounded-md border px-1.5 text-[12px] transition-colors hover:bg-koma-hover disabled:opacity-50 ${enabled ? 'border-koma-accent/40 bg-koma-accent/10 text-koma-accent' : 'border-koma-border bg-koma-panel text-koma-fg'}`}
    onClick={() => {
      if (enabled) {
        useComputerPreview.getState().hide()
        req({ r: 'Computer', action: 'stop' })
      } else {
        useComputerPreview.getState().requestShow(session)
        req({ r: 'Computer', action: 'enable' })
      }
    }}>
    {pending ? <LoaderCircle size={13} className="animate-spin" /> : <Monitor size={13} />}
  </button>
}
