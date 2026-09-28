import { Monitor, Pause, Play, Square, Hand, PictureInPicture2, ExternalLink, Check, Minus } from 'lucide-react'
import { useKoma } from '../store/koma'
import { isComputerScreen } from '../types/computer'
import { useComputerPreview } from '../store/computerPreview'

const button = 'inline-flex items-center gap-2 rounded-md border border-koma-border bg-koma-panel px-3 py-2 text-xs transition-colors hover:bg-koma-hover disabled:opacity-40 disabled:cursor-not-allowed'
const capabilityLabels: Record<string, string> = { capture: 'Screen/window capture', windows: 'Screen and application selection', focus: 'Model source selection', pointer: 'Pointer input', keyboard: 'Keyboard input', accessibility: 'Application accessibility', ocr: 'Text recognition', floating: 'Detached preview' }

export function ComputerSettings() {
  const session = useKoma(s => s.session.id)
  const remote = useKoma(s => s.remoteState.state)
  const computer = useKoma(s => s.computer)
  const error = useKoma(s => s.computerError)
  const req = useKoma(s => s.req)
  const show = useComputerPreview(s => s.show)
  const requestShow = useComputerPreview(s => s.requestShow)
  const enabling = useComputerPreview(s => !!session && s.requestedSession === session)
  const hide = useComputerPreview(s => s.hide)
  const status = computer?.session === session ? computer : null
  const local = !['ready', 'connected', 'connecting'].includes(remote)
  const available = !!session && local
  const enabled = !!status?.enabled
  const observation = status?.observation
  const assist = !!observation && !isComputerScreen(observation.window.id)
  const label = !enabled ? 'Off' : status.paused ? 'Paused' : status.busy ? 'Working' : 'Ready'
  const control = (action: 'enable' | 'pause' | 'resume' | 'stop' | 'take_over') => req({ r: 'Computer', action })

  return <div className="space-y-5 text-[12.5px]">
    <div className="rounded-lg border border-koma-border bg-koma-panel/40 p-4">
      <div className="flex items-center gap-3">
        <div className="rounded-lg border border-koma-border bg-koma-bg p-2.5 text-koma-accent"><Monitor size={20} strokeWidth={1.5} /></div>
        <div className="min-w-0 flex-1"><h3 className="font-medium">Screen and application sharing</h3><p className="mt-1 text-xs text-koma-dim">Enabling Computer use lets the model choose screens and control your mouse and keyboard without per-action approvals. Application views remain read-only; pause or stop sharing to end control.</p></div>
        <span className={`flex items-center gap-1.5 rounded-full border border-koma-border px-2 py-1 text-[10px] ${enabled ? 'text-koma-accent' : 'text-koma-dim'}`}><span className={`h-1.5 w-1.5 rounded-full ${enabled ? 'bg-koma-accent' : 'bg-koma-dim'}`} />{label}</span>
      </div>
      <p className="mt-3 text-xs leading-relaxed text-koma-dim">Model frames fit within 1920 pixels and 2 megapixels. Live previews fit within 1280 × 960. The model can request a fresh close-up for small text on native displays.</p>
      {assist && <p className="mt-3 text-xs text-koma-fg">Assist mode · view only. To act, the model can select a screen while sharing is enabled.</p>}
      <div className="mt-4 flex flex-wrap gap-2">
        {!enabled ? <button type="button" disabled={!available || enabling} className={`${button} text-koma-accent`} onClick={() => { if (session) requestShow(session); control('enable') }}><Monitor size={14} />{enabling ? 'Enabling…' : 'Start sharing'}</button> : <>
          <button type="button" className={button} onClick={() => control(status?.paused ? 'resume' : 'pause')}>{status?.paused ? <Play size={14} /> : <Pause size={14} />}{status?.paused ? assist ? 'Resume assist' : 'Give control' : 'Pause'}</button>
          <button type="button" className={button} onClick={() => control('stop')}><Square size={13} />Stop sharing</button>
          {!status?.paused && !assist && <button type="button" className={button} onClick={() => control('take_over')}><Hand size={14} />Take back control</button>}
        </>}
      </div>
      <p role="status" className="mt-3 text-xs leading-relaxed text-koma-dim">{!available ? 'Computer use is available in a local GUI session.' : status?.message || 'Enable sharing, then choose a screen or application. Application shares are view-only; enabling Computer use authorizes screen control without per-action approvals in every mode.'}</p>
      {error && <p role="alert" className="mt-2 text-xs text-koma-error">{error}</p>}
    </div>

    <div>
      <h3 className="font-medium">Preview</h3>
      <p className="mt-1 text-xs leading-relaxed text-koma-dim">Resize the preview to inspect the shared frame. Hover over it to choose a screen or application. Live frames refresh while the preview is open; model observations are captured separately.</p>
      <div className="mt-3 flex flex-wrap gap-2">
        <button type="button" className={button} disabled={!available || !enabled} onClick={() => { if (session) show(session) }}><PictureInPicture2 size={14} />Open preview</button>
        {status?.capabilities.floating && <button type="button" className={button} disabled={!enabled || status.busy} onClick={() => { hide(); window.ipc?.postMessage(JSON.stringify({ t: 'win', a: 'computer-viewer' })) }}><ExternalLink size={14} />Detach preview</button>}
      </div>
      <p className="mt-2 text-xs text-koma-dim">Detached previews step out of the way during desktop operations. Hiding a preview keeps control enabled.</p>
      {observation && <div className="mt-3 rounded-md border border-koma-border px-3 py-2 text-xs"><p className="truncate">{observation.window.title || observation.window.application}</p><p className="mt-1 text-koma-dim">{observation.transform.width} × {observation.transform.height} · Last shared {new Date(observation.captured_ms).toLocaleTimeString()}</p></div>}
    </div>

    {status && <div>
      <h3 className="font-medium">Capabilities &amp; extraction</h3>
      <dl className="mt-3 grid grid-cols-1 gap-x-6 sm:grid-cols-2">
        {Object.entries(capabilityLabels).map(([key, title]) => {
          const supported = status.capabilities[key as keyof typeof status.capabilities] === true
          return <div key={key} className="flex items-center justify-between border-b border-koma-border/60 py-2 text-xs"><dt>{title}</dt><dd className={`flex items-center gap-1 ${supported ? 'text-koma-accent' : 'text-koma-dim'}`}>{supported ? <Check size={12} /> : <Minus size={12} />}{supported ? 'Available' : 'Unavailable'}</dd></div>
        })}
      </dl>
      <div className="mt-3 space-y-2 text-xs leading-relaxed text-koma-dim">
        {status.capabilities.limitations.map(message => <p key={message}>{message}</p>)}
        {observation && <><p><span className="text-koma-fg">Accessibility: </span>{observation.accessibility_status}</p><p><span className="text-koma-fg">Text recognition: </span>{observation.ocr_status}</p></>}
      </div>
    </div>}
    <p className="text-xs leading-relaxed text-koma-dim">Pause and Take back control cancel queued input while the live share stays open. Give control resumes with a fresh model observation. Stop sharing ends control and closes the preview. Completed actions remain.</p>
  </div>
}
