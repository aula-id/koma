import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { createPortal } from 'react-dom'
import { Minus, Plus } from 'lucide-react'
import {
  cacheDesignImage,
  defaultGradient,
  designImageUrl,
  hashBytes,
  hexToHsb,
  hexToRgba,
  hsbToHex,
  imagePaint,
  mimeForName,
  nodeFillCss,
  resolveColor,
  rgbaToHex,
  solidPaint,
  type DesignDoc,
  type DesignGradientKind,
  type DesignImageScale,
  type DesignPaint,
} from '../../lib/design'
import { TokenMenu } from './DesignTokenMenu'
import type { DesignToken } from '../../lib/design'

const KINDS: { value: DesignGradientKind; label: string }[] = [
  { value: 'linear', label: 'Linear' },
  { value: 'radial', label: 'Radial' },
  { value: 'angular', label: 'Angular' },
  { value: 'diamond', label: 'Diamond' },
]

function colorInputHex(value: string, fallback: string): string {
  const source = /^#[0-9a-fA-F]{3,8}$/.test(value) ? value : fallback
  const body = source.slice(1)
  if (/^[0-9a-fA-F]{6}$/.test(body) || /^[0-9a-fA-F]{8}$/.test(body)) return `#${body.slice(0, 6).toLowerCase()}`
  if (/^[0-9a-fA-F]{3}$/.test(body)) return `#${body[0]}${body[0]}${body[1]}${body[1]}${body[2]}${body[2]}`.toLowerCase()
  return '#000000'
}

function rampCss(stops: { color: string; at: number }[]): string {
  const sorted = stops.slice().sort((a, b) => a.at - b.at)
  if (!sorted.length) return 'transparent'
  return `linear-gradient(90deg, ${sorted.map((stop) => `${stop.color} ${Math.round(stop.at * 1000) / 10}%`).join(', ')})`
}

function parseHex(raw: string, tokens?: DesignToken[]): string | null {
  const trimmed = raw.trim()
  if (tokens?.some((token) => token.name === trimmed)) return trimmed
  const body = trimmed.startsWith('#') ? trimmed.slice(1) : trimmed
  if (/^[0-9a-fA-F]{3}$/.test(body)) return `#${body.split('').map((item) => item + item).join('')}`.toLowerCase()
  if (/^[0-9a-fA-F]{6}$/.test(body)) return `#${body}`.toLowerCase()
  return null
}

export function DesignPaintPopup({
  doc,
  paint,
  fallback,
  tokens,
  allowGradient = true,
  allowImage = true,
  anchor,
  onChange,
  onClose,
  onStoreImage,
}: {
  doc: DesignDoc
  paint: DesignPaint
  fallback: string
  tokens?: DesignToken[]
  allowGradient?: boolean
  allowImage?: boolean
  anchor: HTMLElement | null
  onChange: (paint: DesignPaint) => void
  onClose: () => void
  onStoreImage?: (hash: string, bytes: Uint8Array, mime: string) => void
}) {
  const panelRef = useRef<HTMLDivElement>(null)
  const fileRef = useRef<HTMLInputElement>(null)
  const [editing, setEditing] = useState(0)
  const mode = paint.type === 'gradient' ? 'gradient' : paint.type === 'image' ? 'image' : 'solid'
  const stops = paint.stops?.length ? paint.stops : [{ color: '#4f46e5', at: 0 }, { color: '#22d3ee', at: 1 }]
  const stop = stops[Math.min(editing, stops.length - 1)] ?? stops[0]
  const solid = paint.color && paint.color !== 'none' ? paint.color : fallback
  const hex = mode === 'gradient' ? stop?.color ?? fallback : solid
  const picker = colorInputHex(hex, fallback)
  const imageUrl = paint.hash ? designImageUrl(paint.hash) : ''
  const opacity = Math.round((paint.opacity ?? 1) * 100)
  const rect = anchor?.getBoundingClientRect()
  const width = 284
  const left = rect ? Math.min(Math.max(8, rect.left), window.innerWidth - width - 8) : 8
  const top = rect ? (rect.bottom + 8 + 510 > window.innerHeight ? Math.max(8, rect.top - 518) : rect.bottom + 8) : 8

  useEffect(() => {
    const onDoc = (event: MouseEvent) => {
      const target = event.target as Node
      if (panelRef.current?.contains(target) || anchor?.contains(target)) return
      onClose()
    }
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('mousedown', onDoc)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onDoc)
      window.removeEventListener('keydown', onKey)
    }
  }, [anchor, onClose])

  const setSolid = (color: string) => onChange({ ...solidPaint(color), opacity: paint.opacity })
  const setStops = (next: { color: string; at: number }[], kind = paint.kind ?? 'linear') => {
    onChange({ type: 'gradient', kind, stops: next, opacity: paint.opacity })
  }
  const setOpacity = (value: number) => {
    const next = Math.max(0, Math.min(100, value)) / 100
    onChange({ ...paint, opacity: next >= 1 ? undefined : next })
  }
  const takeImage = async (file: File) => {
    const mime = file.type || mimeForName(file.name)
    if (!mime?.startsWith('image/')) return
    const bytes = new Uint8Array(await file.arrayBuffer())
    const hash = await hashBytes(bytes)
    cacheDesignImage(hash, bytes, mime)
    onStoreImage?.(hash, bytes, mime)
    onChange(imagePaint(hash, paint.scale ?? 'fill'))
  }
  const applyHex = (raw: string) => {
    const color = parseHex(raw, tokens)
    if (!color) return
    if (mode === 'gradient') setStops(stops.map((item, index) => (index === editing ? { ...item, color } : item)))
    else setSolid(color)
  }

  return createPortal(
    <div
      ref={panelRef}
      role="dialog"
      aria-label="Color"
      className="fixed z-[80] flex w-[284px] flex-col gap-2 rounded-lg border border-koma-border bg-koma-panel p-3 shadow-[0_0_12px_rgba(0,0,0,0.35)]"
      style={{ left, top }}
    >
      <select
        aria-label="Fill type"
        value={mode}
        onChange={(event) => {
          const next = event.target.value
          if (next === 'solid') setSolid(solid.startsWith('#') ? solid : fallback)
          else if (next === 'gradient') onChange(defaultGradient(paint.kind ?? 'linear'))
          else if (next === 'image') {
            if (paint.hash) onChange({ ...paint, type: 'image', hash: paint.hash, scale: paint.scale ?? 'fill' })
            else fileRef.current?.click()
          }
        }}
        className="h-8 w-[116px] rounded-lg bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
      >
        <option value="solid">Solid</option>
        {allowGradient ? <option value="gradient">Gradient</option> : null}
        {allowImage ? <option value="image">Image</option> : null}
      </select>
      {mode === 'image' ? (
        <div className="flex flex-col">
          <div
            className="mb-1.5 flex h-[140px] items-center justify-center overflow-hidden rounded-lg"
            style={{
              backgroundColor: '#2a2d37',
              backgroundImage: 'conic-gradient(#3a3d48 0 25%, #2a2d37 0 50%, #3a3d48 0 75%, #2a2d37 0)',
              backgroundSize: '16px 16px',
            }}
          >
            {imageUrl ? <img src={imageUrl} alt="" className="max-h-full max-w-full object-contain" /> : null}
          </div>
          {paint.hash ? (
            <div className="mb-2 grid grid-cols-4 gap-0.5">
              {(['fill', 'fit', 'crop', 'tile'] as DesignImageScale[]).map((scale) => (
                <button
                  key={scale}
                  type="button"
                  title={scale}
                  aria-pressed={(paint.scale ?? 'fill') === scale}
                  onClick={() => onChange({ ...paint, type: 'image', scale })}
                  className={`h-8 rounded-lg text-[11px] capitalize ${((paint.scale ?? 'fill') === scale) ? 'bg-koma-hover text-koma-fg' : 'text-koma-dim hover:bg-koma-hover'}`}
                >
                  {scale}
                </button>
              ))}
            </div>
          ) : null}
          <button
            type="button"
            className="h-8 w-full rounded-lg bg-koma-bg text-[12px] text-koma-fg hover:bg-koma-hover"
            onClick={() => fileRef.current?.click()}
          >
            Choose image
          </button>
        </div>
      ) : null}
      {mode === 'gradient' ? (
        <div className="flex flex-col gap-2">
          <select
            aria-label="Gradient kind"
            value={paint.kind ?? 'linear'}
            onChange={(event) => setStops(stops, event.target.value as DesignGradientKind)}
            className="h-8 w-full rounded-lg bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
          >
            {KINDS.map((kind) => (
              <option key={kind.value} value={kind.value}>{kind.label}</option>
            ))}
          </select>
          <GradientRamp stops={stops} selected={editing} onSelect={setEditing} onStops={(next) => setStops(next)} />
        </div>
      ) : null}
      {mode === 'solid' || mode === 'gradient' ? (
        <div className="flex flex-col gap-2">
          <HsvaWell hex={picker} onChange={applyHex} />
          {picker.startsWith('#') ? (
            <div className="grid grid-cols-[16px_1fr] items-center gap-1 text-[11px] text-koma-dim">
              {(['h', 's', 'b'] as const).map((key) => {
                const hsb = hexToHsb(picker)
                const max = key === 'h' ? 360 : 100
                return (
                  <span key={key} className="contents">
                    <span className="uppercase">{key}</span>
                    <input
                      type="range"
                      aria-label={key === 'h' ? 'Hue' : key === 's' ? 'Saturation' : 'Brightness'}
                      min={0}
                      max={max}
                      value={Math.round(hsb[key])}
                      onChange={(event) => {
                        const next = { ...hsb, [key]: Number(event.target.value) }
                        applyHex(hsbToHex(next.h, next.s, next.b))
                      }}
                    />
                  </span>
                )
              })}
            </div>
          ) : null}
          <button
            type="button"
            className="h-7 rounded-lg bg-koma-bg text-[12px] text-koma-fg hover:bg-koma-hover"
            onClick={async () => {
              const Eye = (window as Window & { EyeDropper?: new () => { open: () => Promise<{ sRGBHex: string }> } }).EyeDropper
              if (!Eye) return
              try {
                const result = await new Eye().open()
                applyHex(result.sRGBHex)
              } catch {
                /* cancelled */
              }
            }}
          >
            Eyedropper
          </button>
          <div className="flex gap-1">
            <div className="flex h-8 min-w-0 flex-1 items-center rounded-lg bg-koma-bg px-2 focus-within:outline focus-within:outline-1 focus-within:outline-koma-accent">
              <input
                aria-label="Hex"
                defaultValue={hex.startsWith('#') ? hex.slice(1).toUpperCase() : hex}
                key={hex}
                onBlur={(event) => applyHex(event.target.value)}
                className="h-8 min-w-0 flex-1 bg-transparent text-[12px] text-koma-fg outline-none"
              />
            </div>
            <div className="flex h-8 w-[60px] flex-none items-center rounded-lg bg-koma-bg px-1.5 focus-within:outline focus-within:outline-1 focus-within:outline-koma-accent">
              <span className="text-[11px] text-koma-dim">%</span>
              <input
                aria-label="Opacity"
                defaultValue={String(opacity)}
                key={opacity}
                onBlur={(event) => {
                  const value = Number(event.target.value)
                  if (Number.isFinite(value)) setOpacity(value)
                }}
                className="h-8 min-w-0 flex-1 bg-transparent pl-1 text-[12px] text-koma-fg outline-none"
              />
            </div>
          </div>
          {picker.startsWith('#') ? (
            <div className="grid grid-cols-4 gap-1">
              {(['r', 'g', 'b'] as const).map((key) => {
                const rgba = hexToRgba(picker)
                return (
                  <label key={key} className="flex h-8 items-center gap-1 rounded-lg bg-koma-bg px-1.5">
                    <span className="text-[11px] uppercase text-koma-dim">{key}</span>
                    <input
                      aria-label={key.toUpperCase()}
                      defaultValue={String(rgba[key])}
                      key={`${key}-${rgba[key]}`}
                      onBlur={(event) => {
                        const value = Number(event.target.value)
                        if (!Number.isFinite(value)) return
                        applyHex(rgbaToHex(key === 'r' ? value : rgba.r, key === 'g' ? value : rgba.g, key === 'b' ? value : rgba.b))
                      }}
                      className="h-7 min-w-0 flex-1 bg-transparent text-[12px] text-koma-fg outline-none"
                    />
                  </label>
                )
              })}
              <label className="flex h-8 items-center gap-1 rounded-lg bg-koma-bg px-1.5">
                <span className="text-[11px] text-koma-dim">A</span>
                <input
                  aria-label="Alpha"
                  defaultValue={String(opacity)}
                  key={`a-${opacity}`}
                  onBlur={(event) => {
                    const value = Number(event.target.value)
                    if (Number.isFinite(value)) setOpacity(value)
                  }}
                  className="h-7 min-w-0 flex-1 bg-transparent text-[12px] text-koma-fg outline-none"
                />
              </label>
            </div>
          ) : null}
          <TokenMenu tokens={tokens ?? []} selected={paint.color} onPick={(name) => { if (name) applyHex(name) }} />
        </div>
      ) : null}
      <input
        ref={fileRef}
        type="file"
        accept="image/png,image/jpeg,image/webp,image/gif,image/svg+xml"
        hidden
        onChange={(event) => {
          const file = event.target.files?.[0]
          event.target.value = ''
          if (file) void takeImage(file)
        }}
      />
    </div>,
    document.body,
  )
}

function HsvaWell({ hex, onChange }: { hex: string; onChange: (hex: string) => void }) {
  const wellRef = useRef<HTMLDivElement>(null)
  const hsb = hexToHsb(hex)
  const hue = hsbToHex(hsb.h, 100, 100)
  const pick = (clientX: number, clientY: number) => {
    const box = wellRef.current?.getBoundingClientRect()
    if (!box || box.width <= 0 || box.height <= 0) return
    const s = Math.max(0, Math.min(100, ((clientX - box.left) / box.width) * 100))
    const b = Math.max(0, Math.min(100, (1 - (clientY - box.top) / box.height) * 100))
    onChange(hsbToHex(hsb.h, s, b))
  }
  return (
    <div
      ref={wellRef}
      role="slider"
      aria-label="Saturation and brightness"
      className="relative h-[140px] cursor-crosshair overflow-hidden rounded-lg"
      style={{ background: `linear-gradient(to top, #000, transparent), linear-gradient(to right, #fff, ${hue})` }}
      onPointerDown={(event) => {
        event.currentTarget.setPointerCapture(event.pointerId)
        pick(event.clientX, event.clientY)
      }}
      onPointerMove={(event) => {
        if (!event.currentTarget.hasPointerCapture(event.pointerId)) return
        pick(event.clientX, event.clientY)
      }}
    >
      <span
        className="pointer-events-none absolute h-3 w-3 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-white shadow"
        style={{ left: `${hsb.s}%`, top: `${100 - hsb.b}%`, background: hex }}
      />
    </div>
  )
}

function GradientRamp({
  stops,
  selected,
  onSelect,
  onStops,
}: {
  stops: { color: string; at: number }[]
  selected: number
  onSelect: (index: number) => void
  onStops: (stops: { color: string; at: number }[]) => void
}) {
  const barRef = useRef<HTMLDivElement>(null)
  const drag = useRef<{ index: number } | null>(null)

  const atFromEvent = (clientX: number) => {
    const box = barRef.current?.getBoundingClientRect()
    if (!box || box.width <= 0) return 0
    return Math.max(0, Math.min(1, (clientX - box.left) / box.width))
  }

  return (
    <div className="flex flex-col gap-1">
      <div
        ref={barRef}
        className="relative h-8 cursor-copy rounded-lg"
        style={{ background: rampCss(stops.map((stop) => ({ ...stop, color: stop.color.startsWith('#') ? stop.color : '#888888' }))) }}
        onPointerDown={(event: ReactPointerEvent<HTMLDivElement>) => {
          if ((event.target as HTMLElement).dataset.stop != null) return
          const at = Math.round(atFromEvent(event.clientX) * 100) / 100
          onStops([...stops, { color: stops[selected]?.color ?? '#ffffff', at }])
          onSelect(stops.length)
        }}
      >
        {stops.map((stop, index) => (
          <button
            key={`${stop.color}-${index}`}
            type="button"
            data-stop={index}
            title={`${Math.round(stop.at * 100)}%`}
            aria-label={`Stop ${index + 1}`}
            aria-pressed={selected === index}
            onPointerDown={(event) => {
              event.stopPropagation()
              event.currentTarget.setPointerCapture(event.pointerId)
              drag.current = { index }
              onSelect(index)
            }}
            onPointerMove={(event) => {
              if (!drag.current) return
              const nextAt = Math.round(atFromEvent(event.clientX) * 100) / 100
              onStops(stops.map((item, at) => (at === drag.current?.index ? { ...item, at: nextAt } : item)))
            }}
            onPointerUp={() => { drag.current = null }}
            className={`absolute top-1/2 h-4 w-4 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 ${selected === index ? 'border-koma-accent' : 'border-white'}`}
            style={{ left: `${stop.at * 100}%`, background: stop.color.startsWith('#') ? stop.color : '#888888' }}
          />
        ))}
      </div>
      <div className="flex items-center justify-end gap-1">
        <button type="button" title="Add stop" aria-label="Add stop" className="flex h-8 w-8 items-center justify-center rounded-lg text-koma-dim hover:bg-koma-hover hover:text-koma-fg" onClick={() => { onStops([...stops, { color: '#ffffff', at: 1 }]); onSelect(stops.length) }}>
          <Plus size={14} />
        </button>
        {stops.length > 2 ? (
          <button type="button" title="Remove stop" aria-label="Remove stop" className="flex h-8 w-8 items-center justify-center rounded-lg text-koma-dim hover:bg-koma-hover hover:text-koma-fg" onClick={() => { onStops(stops.filter((_, index) => index !== selected)); onSelect(0) }}>
            <Minus size={14} />
          </button>
        ) : null}
      </div>
    </div>
  )
}

export function paintSwatch(doc: DesignDoc, paint: DesignPaint, fallback: string): string {
  if (paint.type === 'gradient') return nodeFillCss(doc, { id: 'swatch', kind: 'rect', x: 0, y: 0, w: 1, h: 1, fills: [paint] }, fallback)
  if (paint.type === 'image') {
    const url = designImageUrl(paint.hash)
    return url ? `center / cover no-repeat url("${url}")` : 'transparent'
  }
  if (paint.color && paint.color !== 'none') return resolveColor(doc, paint.color, fallback) || fallback
  return 'transparent'
}

export function paintRowLabel(paint: DesignPaint): string {
  if (paint.type === 'image') return 'Image'
  if (paint.type === 'gradient') {
    if (paint.kind === 'radial') return 'Radial'
    if (paint.kind === 'angular') return 'Angular'
    if (paint.kind === 'diamond') return 'Diamond'
    return 'Linear'
  }
  if (paint.color && paint.color !== 'none' && !paint.color.startsWith('#')) return paint.color
  return ''
}
