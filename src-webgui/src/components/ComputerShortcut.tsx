import { LoaderCircle, Monitor, Hand, Play, Square } from 'lucide-react'
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
  const paused = enabled && computer.paused
  const label = pending ? 'Starting desktop sharing…' : enabled ? `Show shared desktop: ${computer.observation?.window.title || 'choose a display'}` : 'Share desktop'
  const controlClass = 'pointer-events-auto flex h-[22px] items-center rounded-md border border-koma-border bg-koma-panel px-1.5 text-koma-fg hover:bg-koma-hover'
  return <div className="flex flex-none items-center gap-1" role="group" aria-label="Desktop sharing">
    <button type="button" aria-label={label} title={label} aria-pressed={enabled} disabled={pending}
    className={`pointer-events-auto flex h-[22px] flex-none items-center gap-1 rounded-md border px-1.5 text-[11px] transition-colors hover:bg-koma-hover disabled:opacity-50 ${enabled ? 'border-koma-accent/40 bg-koma-accent/10 text-koma-accent' : 'border-koma-border bg-koma-panel text-koma-fg'}`}
    onClick={() => {
      if (enabled) {
        useComputerPreview.getState().show(session)
      } else {
        useComputerPreview.getState().requestShow(session)
        req({ r: 'Computer', action: 'enable' })
      }
    }}>
    {pending ? <LoaderCircle size={13} className="animate-spin" /> : <Monitor size={13} />}
    {enabled && <span className="hidden lg:inline">{!computer.observation ? 'Choose display' : paused ? 'Sharing · paused' : 'Sharing'}</span>}
  </button>
  {enabled && <>
    <button type="button" className={controlClass} aria-label={paused ? 'Give control' : 'Take back control'} title={paused ? 'Give control to the agent' : 'Take back control; sharing stays live'} onClick={() => req({ r: 'Computer', action: paused ? 'resume' : 'take_over' })}>{paused ? <Play size={12} /> : <Hand size={12} />}</button>
    <button type="button" className={controlClass} aria-label="Stop sharing" title="Stop sharing" onClick={() => { useComputerPreview.getState().hide(); req({ r: 'Computer', action: 'stop' }) }}><Square size={11} /></button>
  </>}
  </div>
}
