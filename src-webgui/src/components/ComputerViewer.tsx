import { useEffect, useState } from 'react'
import type { ComputerStatus } from '../types/computer'

/** Native floating window: only observation data and stop/pause controls. */
export function ComputerViewer() {
  const [status, setStatus] = useState<ComputerStatus | null>(window.__komaComputerInitial ?? null)
  const [overlays, setOverlays] = useState(false)
  useEffect(() => {
    setStatus(window.__komaComputerInitial ?? null)
    const receive = (event: Event) => setStatus((event as CustomEvent<ComputerStatus>).detail)
    const apply = (palette: Record<string, string>) => {
      for (const key of ['bg', 'fg', 'dim', 'accent', 'panel', 'warn', 'success', 'info', 'error']) {
        const color = palette[key]
        if (typeof color === 'string' && /^#[0-9a-fA-F]{6}$/.test(color)) document.documentElement.style.setProperty(`--koma-${key}`, color)
      }
    }
    apply(window.__komaComputerPalette ?? {})
    const palette = (event: Event) => apply((event as CustomEvent<Record<string, string>>).detail)
    window.addEventListener('koma-computer-palette', palette)
    window.addEventListener('koma-computer', receive)
    return () => { window.removeEventListener('koma-computer', receive); window.removeEventListener('koma-computer-palette', palette) }
  }, [])
  const observation = status?.observation
  const control = (action: string) => window.ipc?.postMessage(JSON.stringify({ action }))
  const imageUrl = observation ? `${location.protocol === 'http:' || location.protocol === 'https:' ? location.origin : 'koma://localhost'}/image/${encodeURIComponent(observation.image_path)}` : ''
  return <main className="flex h-screen flex-col overflow-auto bg-koma-panel p-2 text-xs text-koma-fg">
    <div className="flex shrink-0 items-center gap-2 pb-2">
      <strong className="flex-1 truncate">{observation?.window.title ?? 'Computer observation'}</strong>
      {status?.enabled && <>
        <button onClick={() => control(status.paused ? 'resume' : 'pause')}>{status.paused ? 'Resume' : 'Pause'}</button>
        <button onClick={() => control('stop')}>Stop</button>
        <button onClick={() => control('take_over')}>Take over</button>
      </>}
    </div>
    <p role="status" className="shrink-0 pb-2 text-koma-dim">{status?.message ?? 'Select a window in Koma.'}{observation && ` · ${new Date(observation.captured_ms).toLocaleTimeString()}`}</p>
    {observation && <div className="min-h-0 flex-1 overflow-auto">
      <div className="relative w-full">
        <img src={imageUrl} alt={`Observation of ${observation.window.title}`} className="block w-full" draggable={false} />
        {overlays && observation.elements.map(element => <span key={element.id} title={`${element.source}: ${element.label}`} className={`absolute border ${element.source === 'ocr' ? 'border-amber-400' : 'border-sky-400'}`}
          style={{ left: `${100 * element.bounds.x / observation.transform.width}%`, top: `${100 * element.bounds.y / observation.transform.height}%`, width: `${100 * element.bounds.width / observation.transform.width}%`, height: `${100 * element.bounds.height / observation.transform.height}%` }} />)}
      </div>
    </div>}
    <label className="flex shrink-0 gap-2 pt-2"><input type="checkbox" checked={overlays} onChange={e => setOverlays(e.target.checked)} />Accessibility / OCR overlays</label>
  </main>
}
