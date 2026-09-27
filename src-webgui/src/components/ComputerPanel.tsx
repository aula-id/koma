import { useEffect, useRef, useState, type PointerEvent } from 'react'
import { GripHorizontal, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import { useComputerPreview } from '../store/computerPreview'
import { ComputerPreview } from './ComputerPreview'

const preference = 'koma.computer.preview'
type Bounds = { x: number; y: number; width: number; height: number }
function bounded(value: Partial<Bounds>): Bounds {
  const width = Math.max(300, Math.min(Number.isFinite(value.width) ? value.width! : 560, window.innerWidth - 24))
  const height = Math.max(240, Math.min(Number.isFinite(value.height) ? value.height! : 360, window.innerHeight - 70))
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
  const req = useKoma(s => s.req)
  const previewSession = useComputerPreview(s => s.session)
  const hide = useComputerPreview(s => s.hide)
  const open = !!session && previewSession === session
  const [bounds, setBounds] = useState(initialBounds)
  const panel = useRef<HTMLElement>(null)
  const drag = useRef<{ x: number; y: number; left: number; top: number } | null>(null)
  const current = status?.session === session ? status : null
  const local = remote !== 'ready' && remote !== 'connected' && remote !== 'connecting'
  const control = (action: 'enable' | 'windows' | 'select' | 'pause' | 'resume' | 'stop' | 'take_over', window?: string) => req({ r: 'Computer', action, window })

  useEffect(() => {
    if (previewSession && previewSession !== session) hide()
  }, [session, previewSession, hide])
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
  if (!session || !local || !open) return null
  return <section ref={panel} role="dialog" aria-label="Shared window preview"
    className="fixed z-40 rounded-xl border border-koma-border bg-koma-panel shadow-2xl"
    style={{ left: bounds.x, top: bounds.y, width: bounds.width, height: bounds.height, minWidth: 300, minHeight: 240, maxWidth: 'calc(100vw - 24px)', maxHeight: 'calc(100vh - 70px)', resize: 'both', overflow: 'hidden' }}>
    <ComputerPreview status={current} control={control} chrome={<>
      <span role="img" aria-label="Drag preview" title="Drag preview" className="cursor-move touch-none rounded-md p-1.5 text-koma-dim hover:bg-koma-hover"
        onPointerDown={startDrag} onPointerMove={moveDrag} onPointerUp={() => { drag.current = null }} onPointerCancel={() => { drag.current = null }}><GripHorizontal size={15} /></span>
      <button type="button" aria-label="Hide preview" title="Hide preview (control stays enabled)" onClick={hide} className="rounded-md p-1.5 text-koma-dim hover:bg-koma-hover"><X size={14} /></button>
    </>} />
  </section>
}
