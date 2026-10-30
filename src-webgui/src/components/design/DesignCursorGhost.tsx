import { useEffect, useRef, type RefObject } from 'react'
import { PenTool, Type } from 'lucide-react'
import { canvasPoint } from '../../lib/uiScale'

/**
 * What the miniature looks like. The pen and text tools have no fixed size, so they
 * show their glyph and a hint instead.
 */
export type CursorGhostShape = 'frame' | 'rect' | 'ellipse' | 'triangle' | 'polygon' | 'star' | 'line' | 'pen' | 'text'

/** Outline of a shape miniature in a w×h box, as an SVG path. */
function ghostPath(shape: CursorGhostShape, w: number, h: number): string {
  const poly = (points: [number, number][]) => `M${points.map(([x, y]) => `${x} ${y}`).join(' L')} Z`
  const regular = (count: number, inner?: number) => {
    const points: [number, number][] = []
    const steps = inner ? count * 2 : count
    for (let i = 0; i < steps; i++) {
      const radius = inner && i % 2 ? inner : 1
      const angle = -Math.PI / 2 + (i * 2 * Math.PI) / steps
      points.push([w / 2 + (Math.cos(angle) * radius * w) / 2, h / 2 + (Math.sin(angle) * radius * h) / 2])
    }
    return poly(points)
  }
  if (shape === 'ellipse') return `M0 ${h / 2} A${w / 2} ${h / 2} 0 1 0 ${w} ${h / 2} A${w / 2} ${h / 2} 0 1 0 0 ${h / 2} Z`
  if (shape === 'triangle') return regular(3)
  if (shape === 'polygon') return regular(6)
  if (shape === 'star') return regular(5, 0.38)
  if (shape === 'line') return `M0 ${h / 2} H${w}`
  return `M0 0 H${w} V${h} H0 Z`
}

/** Longest side of the miniature that rides with the cursor. */
const GHOST_MAX = 96
/** Gap between the cursor tip and the miniature, so it never covers the click point. */
const GHOST_OFFSET = 14
/** Tilt per pixel of movement in one frame, and its cap, in degrees. */
const TILT_PER_PX = 0.6
const TILT_MAX = 8
/** Screen pixels a press may travel and still count as a click that places the frame. */
const CLICK_SLOP = 4

/**
 * A downscaled shape that follows the cursor while a drawing tool is armed, so it is
 * obvious that the next click on the canvas places one. It trails the cursor on a spring
 * and tilts toward the motion, then settles upright. A click shows it growing into the
 * real frame at the click point; a drag hides it, since the drag draws the frame itself.
 * Everything runs on the DOM directly, once per animation frame, without React renders.
 */
export function DesignCursorGhost({
  canvasRef,
  shape,
  width,
  height,
  label,
  zoom,
}: {
  canvasRef: RefObject<HTMLDivElement | null>
  shape: CursorGhostShape
  width: number
  height: number
  label: string
  /** Canvas zoom, so the click animation grows to the frame's real on-screen size. */
  zoom: number
}) {
  const ghostRef = useRef<HTMLDivElement>(null)
  const zoomRef = useRef(zoom)
  zoomRef.current = zoom
  const scale = GHOST_MAX / Math.max(width, height, 1)
  const pen = shape === 'pen' || shape === 'text'
  const miniW = pen ? 16 : Math.max(12, Math.round(width * scale))
  const miniH = pen ? 16 : shape === 'line' ? 12 : Math.max(12, Math.round(height * scale))

  useEffect(() => {
    const canvas = canvasRef.current
    const ghost = ghostRef.current
    if (!canvas || !ghost) return
    const still = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false
    const state = { x: 0, y: 0, tx: 0, ty: 0, tilt: 0, lastTx: 0, seen: false, pressed: false, frame: 0, down: { x: 0, y: 0 } }
    const local = (event: PointerEvent) => {
      const rect = canvas.getBoundingClientRect()
      return { x: canvasPoint(event.clientX - rect.left), y: canvasPoint(event.clientY - rect.top) }
    }
    const paint = () => {
      ghost.style.transform = `translate(${state.x + GHOST_OFFSET}px, ${state.y + GHOST_OFFSET}px) rotate(${state.tilt}deg)`
    }
    const step = () => {
      state.frame = 0
      const moved = state.tx - state.lastTx
      state.lastTx = state.tx
      if (still) {
        state.x = state.tx
        state.y = state.ty
        state.tilt = 0
      } else {
        state.x += (state.tx - state.x) * 0.35
        state.y += (state.ty - state.y) * 0.35
        const target = Math.max(-TILT_MAX, Math.min(TILT_MAX, moved * TILT_PER_PX))
        state.tilt += (target - state.tilt) * 0.2
      }
      paint()
      const settled = Math.abs(state.tx - state.x) < 0.1 && Math.abs(state.ty - state.y) < 0.1 && Math.abs(state.tilt) < 0.05
      if (!settled) state.frame = requestAnimationFrame(step)
    }
    const wake = () => {
      if (!state.frame) state.frame = requestAnimationFrame(step)
    }
    const onMove = (event: PointerEvent) => {
      const point = local(event)
      state.tx = point.x
      state.ty = point.y
      if (!state.seen) {
        // The first sighting jumps straight to the cursor instead of flying in from the corner.
        state.seen = true
        state.x = point.x
        state.y = point.y
        state.lastTx = point.x
      }
      if (state.pressed) {
        const far = Math.hypot(point.x - state.down.x, point.y - state.down.y) > CLICK_SLOP
        ghost.style.opacity = far ? '0' : ''
      } else ghost.style.opacity = ''
      wake()
    }
    const onLeave = () => {
      state.seen = false
      ghost.style.opacity = '0'
    }
    const onDown = (event: PointerEvent) => {
      if (event.button !== 0) return
      state.pressed = true
      state.down = local(event)
    }
    const onUp = (event: PointerEvent) => {
      if (!state.pressed) return
      state.pressed = false
      const point = local(event)
      // The pen places a point and text opens an empty line, so there is nothing to grow into.
      if (pen || Math.hypot(point.x - state.down.x, point.y - state.down.y) > CLICK_SLOP || still) return
      // Grow a copy of the miniature into the real frame where the click placed it.
      const drop = ghost.cloneNode(true) as HTMLDivElement
      drop.style.transform = ''
      drop.style.left = `${state.down.x}px`
      drop.style.top = `${state.down.y}px`
      canvas.appendChild(drop)
      const realW = width * zoomRef.current
      const realH = height * zoomRef.current
      const animation = drop.animate(
        [
          { transform: `translate(${GHOST_OFFSET}px, ${GHOST_OFFSET}px) rotate(${state.tilt}deg)`, opacity: 1 },
          { transform: `scale(${realW / miniW}, ${realH / miniH})`, opacity: 0 },
        ],
        { duration: 120, easing: 'ease-out' },
      )
      animation.onfinish = () => drop.remove()
      animation.oncancel = () => drop.remove()
    }
    ghost.style.opacity = '0'
    canvas.addEventListener('pointermove', onMove)
    canvas.addEventListener('pointerleave', onLeave)
    canvas.addEventListener('pointerdown', onDown, true)
    window.addEventListener('pointerup', onUp, true)
    return () => {
      if (state.frame) cancelAnimationFrame(state.frame)
      canvas.removeEventListener('pointermove', onMove)
      canvas.removeEventListener('pointerleave', onLeave)
      canvas.removeEventListener('pointerdown', onDown, true)
      window.removeEventListener('pointerup', onUp, true)
    }
  }, [canvasRef, width, height, miniW, miniH, pen])

  return (
    <div
      ref={ghostRef}
      aria-hidden="true"
      className="pointer-events-none absolute left-0 top-0 z-30 flex flex-col items-start gap-1 transition-opacity duration-100"
      style={{ transformOrigin: '0 0', opacity: 0 }}
    >
      {shape === 'text' ? (
        <Type size={miniW} strokeWidth={2.25} className="text-koma-accent" />
      ) : pen ? (
        <PenTool size={miniW} strokeWidth={2} className="text-koma-accent" />
      ) : (
        <svg width={miniW} height={miniH} viewBox={`-1 -1 ${miniW + 2} ${miniH + 2}`} className="overflow-visible">
          <path
            d={ghostPath(shape, miniW, miniH)}
            fill={shape === 'line' ? 'none' : 'color-mix(in srgb, var(--color-koma-accent) 10%, transparent)'}
            stroke="var(--color-koma-accent)"
            strokeWidth={1}
            strokeDasharray="3 2"
            strokeLinejoin="round"
          />
        </svg>
      )}
      <span className="whitespace-nowrap rounded bg-koma-panel px-1 text-[10px] leading-4 text-koma-fg shadow-sm">{label}</span>
    </div>
  )
}
