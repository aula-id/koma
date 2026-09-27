import { useEffect, useRef, useState, type PointerEvent } from 'react'
import { Monitor, X } from 'lucide-react'
import { useKoma } from '../store/koma'

const preference = 'koma.computer.preview'
type Bounds = { x: number; y: number; width: number; height: number }
function bounded(value: Partial<Bounds>): Bounds {
  const width = Math.max(300, Math.min(Number.isFinite(value.width) ? value.width! : 420, window.innerWidth - 24))
  const height = Math.max(240, Math.min(Number.isFinite(value.height) ? value.height! : 460, window.innerHeight - 70))
  return {
    width, height,
    x: Math.max(12, Math.min(Number.isFinite(value.x) ? value.x! : window.innerWidth - width - 24, window.innerWidth - width - 12)),
    y: Math.max(40, Math.min(Number.isFinite(value.y) ? value.y! : 70, window.innerHeight - height - 12)),
  }
}
function initialBounds(): Bounds {
  try { return bounded(JSON.parse(localStorage.getItem(preference) ?? '{}')) } catch { return bounded({}) }
}

export function ComputerPanel() {
  const session = useKoma(s => s.session.id)
  const remote = useKoma(s => s.remoteState.state)
  const status = useKoma(s => s.computer)
  const error = useKoma(s => s.computerError)
  const req = useKoma(s => s.req)
  const [open, setOpen] = useState(false)
  const [overlays, setOverlays] = useState(false)
  const [bounds, setBounds] = useState(initialBounds)
  const panel = useRef<HTMLElement>(null)
  const drag = useRef<{ x: number; y: number; left: number; top: number } | null>(null)
  const current = status?.session === session ? status : null
  const observation = current?.observation
  const enabled = !!current?.enabled
  const local = remote !== 'ready' && remote !== 'connected' && remote !== 'connecting'
  const control = (action: 'enable' | 'windows' | 'select' | 'pause' | 'resume' | 'stop' | 'take_over', window?: string) => req({ r: 'Computer', action, window })

  useEffect(() => { setOpen(false) }, [session])
  useEffect(() => {
    const resize = () => setBounds(v => bounded(v))
    window.addEventListener('resize', resize)
    return () => window.removeEventListener('resize', resize)
  }, [])
  useEffect(() => {
    try { localStorage.setItem(preference, JSON.stringify(bounds)) } catch { /* Optional local preference. */ }
  }, [bounds])
  useEffect(() => {
    if (!open || !panel.current) return
    const observer = new ResizeObserver(entries => {
      const box = entries[0]?.target.getBoundingClientRect()
      if (box) setBounds(v => Math.abs(v.width - box.width) < 1 && Math.abs(v.height - box.height) < 1 ? v : bounded({ ...v, width: box.width, height: box.height }))
    })
    observer.observe(panel.current)
    return () => observer.disconnect()
  }, [open])

  function startDrag(event: PointerEvent<HTMLElement>) {
    if (event.button !== 0 || (event.target as HTMLElement).closest('button')) return
    drag.current = { x: event.clientX, y: event.clientY, left: bounds.x, top: bounds.y }
    event.currentTarget.setPointerCapture(event.pointerId)
  }
  function moveDrag(event: PointerEvent<HTMLElement>) {
    const start = drag.current
    if (start) setBounds(v => bounded({ ...v, x: start.left + event.clientX - start.x, y: start.top + event.clientY - start.y }))
  }
  if (!session || !local) return null
  const button = 'rounded border border-koma-border px-2 py-1 hover:bg-koma-hover disabled:opacity-40'
  const imageUrl = observation ? `${window.location.protocol === 'http:' || window.location.protocol === 'https:' ? window.location.origin : 'koma://localhost'}/image/${encodeURIComponent(observation.image_path)}` : ''
  return <>
    <button aria-label="Computer control" title="Computer control" onClick={() => setOpen(v => !v)}
      className={`fixed bottom-8 right-4 z-30 flex items-center gap-1 rounded border border-koma-border bg-koma-panel px-2 py-1 text-xs ${enabled ? 'text-koma-accent' : 'text-koma-dim'}`}>
      <Monitor size={14} /> Computer{enabled ? current?.paused ? ' · paused' : ' · enabled' : ''}
    </button>
    {open && <section ref={panel} role="dialog" aria-label="Computer observation" className="fixed z-40 flex flex-col rounded-lg border border-koma-border bg-koma-panel text-xs text-koma-fg shadow-xl"
      style={{ left: bounds.x, top: bounds.y, width: bounds.width, height: bounds.height, minWidth: 300, minHeight: 240, maxWidth: 'calc(100vw - 24px)', maxHeight: 'calc(100vh - 70px)', resize: 'both', overflow: 'auto' }}>
      <header className="flex shrink-0 cursor-move select-none items-center gap-2 border-b border-koma-border p-2" onPointerDown={startDrag} onPointerMove={moveDrag} onPointerUp={() => { drag.current = null }} onPointerCancel={() => { drag.current = null }}>
        <Monitor size={14} /><strong className="flex-1">Computer</strong>
        <button aria-label="Hide preview" title="Hide preview (control remains enabled)" onClick={() => setOpen(false)}><X size={14} /></button>
      </header>
      <div className="flex shrink-0 flex-wrap gap-1 p-2">
        {!enabled ? <button className={button} onClick={() => control('enable')}>Enable computer use</button> : <>
          <button className={button} onClick={() => control(current?.paused ? 'resume' : 'pause')}>{current?.paused ? 'Resume' : 'Pause'}</button>
          <button className={button} onClick={() => control('stop')}>Stop</button>
          <button className={button} onClick={() => control('take_over')}>Take over</button>
          {current?.capabilities.floating && <button className={button} onClick={() => window.ipc?.postMessage(JSON.stringify({t:'win',a:'computer-viewer'}))}>Detach preview</button>}
          <button className={button} disabled={current?.busy || current?.paused || !current?.capabilities.windows} onClick={() => control('windows')}>Refresh windows</button>
          {current?.capabilities.capture && !current.capabilities.windows && <button className={button} disabled={current.busy || current.paused} onClick={() => control('select', 'portal:choose')}>Choose window in system dialog</button>}
        </>}
      </div>
      {enabled && !!current?.windows.length && <label className="px-2 pb-2">Observe window
        <select aria-label="Select window to observe" className="mt-1 w-full rounded border border-koma-border bg-koma-panel p-1" value={observation?.window.id ?? ''} disabled={current.busy || current.paused}
          onChange={e => { if (e.target.value) control('select', e.target.value) }}>
          <option value="">Choose a visible window…</option>
          {current.windows.map(w => <option key={w.id} value={w.id}>{w.title || w.application}</option>)}
        </select>
      </label>}
      <p role="status" className="shrink-0 px-2 pb-2 text-koma-dim">{error || (current?.busy ? 'Working… ' : '') + (current?.message || 'Disabled. Enable to authorize observation; input follows your session approval mode.')}</p>
      {observation && <>
        <div className="px-2 pb-1">{observation.window.title} · {new Date(observation.captured_ms).toLocaleTimeString()}</div>
        <div className="min-h-0 flex-1 overflow-auto px-2">
          <div className="relative w-full" style={{ aspectRatio: `${observation.transform.width}/${observation.transform.height}` }}>
            <img src={imageUrl} alt={`Observation of ${observation.window.title}`} draggable={false} className="block h-auto w-full" />
            {overlays && observation.elements.map(element => <span key={element.id} title={`${element.source}: ${element.role} ${element.label}`}
              className={`pointer-events-auto absolute border ${element.source === 'ocr' ? 'border-amber-400' : 'border-sky-400'}`}
              style={{ left: `${100 * element.bounds.x / observation.transform.width}%`, top: `${100 * element.bounds.y / observation.transform.height}%`, width: `${100 * element.bounds.width / observation.transform.width}%`, height: `${100 * element.bounds.height / observation.transform.height}%` }} />)}
          </div>
        </div>
        <label className="flex shrink-0 items-center gap-2 p-2"><input type="checkbox" checked={overlays} onChange={e => setOverlays(e.target.checked)} />Accessibility and OCR overlays</label>
      </>}
      {current && <details className="shrink-0 border-t border-koma-border p-2 text-koma-dim"><summary>Capabilities and extraction</summary>
        <dl className="grid grid-cols-2 gap-1 py-2">{Object.entries(current.capabilities).filter(([k]) => k !== 'limitations').map(([key, value]) => <div key={key}><dt className="inline">{key}: </dt><dd className="inline">{value ? 'available' : 'unavailable'}</dd></div>)}</dl>
        {current.capabilities.limitations.map(message => <p key={message}>{message}</p>)}
        {observation && <><p>{observation.accessibility_status}</p><p>{observation.ocr_status}</p></>}
        <p>Preview shows the model’s last observation. Stop and Take over cancel future input; completed actions remain.</p>
      </details>}
    </section>}
  </>
}
