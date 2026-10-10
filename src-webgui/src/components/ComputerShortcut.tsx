import { Eye, Hand, LoaderCircle, Monitor, Pause, Play, Square } from 'lucide-react'
import { useKoma } from '../store/koma'
import { isViewOnlySource } from '../types/computer'
import { useComputerPreview } from '../store/computerPreview'

/** Native titlebar shortcut. The glyph is the state; the words stay on hover. */
export function ComputerShortcut() {
  const session = useKoma(s => s.session.id)
  const remote = useKoma(s => s.remoteState.state)
  const computer = useKoma(s => s.computer)
  const req = useKoma(s => s.req)
  const pending = useComputerPreview(s => s.requestedSession === session && !!session)
  const enabled = !!computer?.enabled && computer.session === session
  if (!session || ['ready', 'connected', 'connecting'].includes(remote)) return null
  const assist = !!computer?.observation && isViewOnlySource(computer.observation.window.id, !!computer.capabilities?.pointer, !!computer.capabilities?.keyboard)
  const paused = enabled && computer.paused
  const choosing = enabled && !computer.observation
  const label = pending ? 'Starting sharing' : !enabled ? 'Share a screen' : choosing ? 'Choose a screen' : paused ? 'Sharing paused' : assist ? 'View only' : 'Sharing'
  const Icon = pending ? LoaderCircle : paused ? Pause : assist ? Eye : Monitor
  const controlClass = 'pointer-events-auto flex h-[22px] w-[22px] items-center justify-center rounded-md border border-koma-border bg-koma-panel text-koma-fg hover:bg-koma-hover'
  return <div className="flex flex-none items-center gap-1" role="group" aria-label="Computer sharing">
    <button type="button" aria-label={label} title={label} aria-pressed={enabled} disabled={pending}
    className={`pointer-events-auto flex h-[22px] w-[22px] flex-none items-center justify-center rounded-md border transition-colors hover:bg-koma-hover disabled:opacity-50 ${!enabled || paused ? 'border-koma-border bg-koma-panel text-koma-fg' : 'border-koma-accent/40 bg-koma-accent/10 text-koma-accent'}`}
    onClick={() => {
      if (enabled) {
        useComputerPreview.getState().show(session)
      } else {
        useComputerPreview.getState().requestShow(session)
        req({ r: 'Computer', action: 'enable' })
      }
    }}>
    <Icon size={13} className={pending ? 'animate-spin' : undefined} />
  </button>
  {enabled && <>
    <button type="button" className={controlClass} aria-label={paused ? assist ? 'Resume assist' : 'Give control' : assist ? 'Pause assist' : 'Take back control'} title={assist ? 'Pause or resume view-only assistance' : paused ? 'Give control to the agent' : 'Take back control; sharing stays live'} onClick={() => req({ r: 'Computer', action: paused ? 'resume' : 'take_over' })}>{paused ? <Play size={12} /> : <Hand size={12} />}</button>
    <button type="button" className={controlClass} aria-label="Stop sharing" title="Stop sharing" onClick={() => { useComputerPreview.getState().dismiss(session); req({ r: 'Computer', action: 'stop' }) }}><Square size={11} /></button>
  </>}
  </div>
}
