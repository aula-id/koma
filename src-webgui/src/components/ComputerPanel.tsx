import { useCallback, useEffect, useRef, useState, type PointerEvent } from 'react'
import { GripHorizontal, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import { useComputerPreview } from '../store/computerPreview'
import { ComputerPreview } from './ComputerPreview'
import { containComputerPreview, fitComputerPreview, type PreviewBounds as Bounds } from '../lib/computerPreviewLayout'

const preference = 'koma.computer.preview'
function bounded(value: Partial<Bounds>, aspect: number | null = null): Bounds {
  return fitComputerPreview(value, aspect, { width: window.innerWidth, height: window.innerHeight })
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
  const requestedSession = useComputerPreview(s => s.requestedSession)
  const open = !!session && previewSession === session && !!status?.enabled && status.session === session
  const [bounds, setBounds] = useState(initialBounds)
  const aspect = useRef<number | null>(null)
  const resizing = useRef<{ x: number; y: number; width: number; height: number } | null>(null)
  const drag = useRef<{ x: number; y: number; left: number; top: number } | null>(null)
  const current = status?.session === session ? status : null
  const source = current?.observation?.window.id ?? null
  const activeSource = useRef(source)
  activeSource.current = source
  const local = remote !== 'ready' && remote !== 'connected' && remote !== 'connecting'
  const control = (action: 'enable' | 'windows' | 'select' | 'pause' | 'resume' | 'stop' | 'take_over', window?: string) => req({ r: 'Computer', action, window })

  useEffect(() => {
    if ((previewSession && previewSession !== session) || (requestedSession && requestedSession !== session)) hide()
  }, [session, previewSession, requestedSession, hide])
  useEffect(() => {
    const resize = () => setBounds(v => bounded(v, aspect.current))
    window.addEventListener('resize', resize)
    return () => window.removeEventListener('resize', resize)
  }, [])
  useEffect(() => {
    try { localStorage.setItem(preference, JSON.stringify(bounds)) } catch { /* Optional local preference. */ }
  }, [bounds])
  const imageSize = useCallback((width: number, height: number) => {
    if (!activeSource.current || !width || !height || !Number.isFinite(width / height)) return
    const ratio = width / height
    if (aspect.current !== null && Math.abs(ratio / aspect.current - 1) < 0.005) return
    aspect.current = ratio
    setBounds(v => containComputerPreview(v, ratio, { width: window.innerWidth, height: window.innerHeight }))
  }, [])
  const imageWidth = current?.observation?.transform.width
  const imageHeight = current?.observation?.transform.height
  useEffect(() => {
    if (!source) aspect.current = null
    else if (imageWidth && imageHeight) imageSize(imageWidth, imageHeight)
  }, [source, imageWidth, imageHeight, imageSize])

  function startDrag(event: PointerEvent<HTMLElement>) {
    if (event.button !== 0 || (event.target as HTMLElement).closest('button')) return
    drag.current = { x: event.clientX, y: event.clientY, left: bounds.x, top: bounds.y }
    event.currentTarget.setPointerCapture(event.pointerId)
  }
  function moveDrag(event: PointerEvent<HTMLElement>) {
    const start = drag.current
    if (start) setBounds(v => bounded({ ...v, x: start.left + event.clientX - start.x, y: start.top + event.clientY - start.y }, aspect.current))
  }
  if (!session || !local || !open) return null
  return <section role="dialog" aria-label="Shared computer preview"
    className="computer-preview-panel fixed z-40 rounded-xl bg-koma-panel shadow-2xl ring-1 ring-koma-border"
    style={{ left: bounds.x, top: bounds.y, width: bounds.width, height: bounds.height, overflow: 'hidden' }}>
    <ComputerPreview status={current} control={control} onImageSize={imageSize} chrome={<>
      <span role="img" aria-label="Drag preview" title="Drag preview" className="cursor-move touch-none rounded-md p-1.5 text-koma-dim hover:bg-koma-hover"
        onPointerDown={startDrag} onPointerMove={moveDrag} onPointerUp={() => { drag.current = null }} onPointerCancel={() => { drag.current = null }}><GripHorizontal size={15} /></span>
      <button type="button" aria-label="Hide preview" title="Hide preview (control stays enabled)" onClick={hide} className="rounded-md p-1.5 text-koma-dim hover:bg-koma-hover"><X size={14} /></button>
    </>} />
    <button type="button" aria-label="Resize preview" title="Resize preview; arrow keys also resize" className="absolute bottom-0 right-0 h-5 w-5 cursor-nwse-resize touch-none text-koma-dim focus-visible:outline focus-visible:outline-koma-accent"
      onPointerDown={event => { if (event.button !== 0) return; resizing.current = { x: event.clientX, y: event.clientY, width: bounds.width, height: bounds.height }; event.currentTarget.setPointerCapture(event.pointerId) }}
      onPointerMove={event => {
        const start = resizing.current
        if (!start) return
        const dx = event.clientX - start.x
        const dy = event.clientY - start.y
        const ratio = aspect.current
        setBounds(v => bounded({ ...v,
          width: start.width + (ratio ? Math.abs(dx) > Math.abs(dy * ratio) ? dx : dy * ratio : dx),
          height: start.height + dy,
        }, ratio))
      }}
      onPointerUp={() => { resizing.current = null }} onPointerCancel={() => { resizing.current = null }} onLostPointerCapture={() => { resizing.current = null }}
      onKeyDown={event => {
        if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) return
        event.preventDefault()
        const direction = event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1 : -1
        const delta = direction * (event.shiftKey ? 50 : 10)
        const vertical = event.key === 'ArrowUp' || event.key === 'ArrowDown'
        setBounds(v => bounded({ ...v,
          width: v.width + (aspect.current || !vertical ? delta : 0),
          height: v.height + (vertical ? delta : 0),
        }, aspect.current))
      }}><svg viewBox="0 0 20 20" className="h-full w-full" aria-hidden="true"><path d="M9 16 16 9M13 16l3-3" fill="none" stroke="currentColor" strokeWidth="1.5" /></svg></button>
  </section>
}
