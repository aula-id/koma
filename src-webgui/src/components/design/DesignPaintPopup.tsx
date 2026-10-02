import { createContext, useContext, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react'
import { ChevronLeft, Minus, Plus } from 'lucide-react'
import {
  cacheDesignImage,
  clearImageCrop,
  defaultGradient,
  designImageUrl,
  ensureImageCrop,
  hashBytes,
  hexToHsb,
  hexToRgba,
  hsbToHex,
  imageNaturalSize,
  imagePaint,
  mimeForName,
  nodeFillCss,
  rememberImageSize,
  resolveColor,
  rgbaToHex,
  solidPaint,
  type DesignDoc,
  type DesignGradientKind,
  type DesignImageScale,
  type DesignPaint,
  type DesignToken,
} from '../../lib/design'
import { KomaSelect } from '../KomaSelect'
import { TokenMenu } from './DesignTokenMenu'
import { useDraftNumber } from './draftInput'

function LabeledControl({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="flex h-8 min-w-0 items-center gap-2 rounded-lg bg-koma-bg px-2">
      <span className="w-14 flex-none truncate text-[11px] text-koma-dim">{label}</span>
      <div className="min-w-0 flex-1">{children}</div>
    </label>
  )
}

const KINDS: { value: DesignGradientKind; label: string }[] = [
  { value: 'linear', label: 'Linear' },
  { value: 'radial', label: 'Radial' },
  { value: 'angular', label: 'Angular' },
  { value: 'diamond', label: 'Diamond' },
]

export type PaintInspectorPage = {
  kind: 'paint'
  title: string
  paint: DesignPaint
  fallback: string
  tokens?: DesignToken[]
  allowImage?: boolean
  allowGradient?: boolean
  box?: { w: number; h: number }
  natural?: { w: number; h: number } | null
  stroke?: boolean
  onChange: (paint: DesignPaint) => void
  onStoreImage?: (hash: string, bytes: Uint8Array, mime: string) => void
}

export type TokenInspectorPage = {
  kind: 'token'
  title: string
  tokens: DesignToken[]
  selected?: string
  onPick: (name: string | null) => void
}

export type DetailInspectorPage = {
  kind: 'shadow' | 'blur' | 'interaction' | 'text'
  title: string
  index?: number
}

export type InspectorPage = PaintInspectorPage | TokenInspectorPage | DetailInspectorPage

export const InspectorPageContext = createContext<(page: InspectorPage) => void>(() => {})

export function useInspectorPage() {
  return useContext(InspectorPageContext)
}

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

export function InspectorBack({ title, onBack }: { title: string; onBack: () => void }) {
  return (
    <div className="flex h-8 items-center gap-1">
      <button
        type="button"
        title="Back"
        aria-label="Back"
        onClick={onBack}
        className="flex h-8 w-8 flex-none items-center justify-center rounded-lg text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
      >
        <ChevronLeft size={16} />
      </button>
      <span className="min-w-0 flex-1 truncate text-[12px] font-medium text-koma-fg">{title}</span>
    </div>
  )
}

export function InspectorPageView({ page, onBack }: { page: InspectorPage; onBack: () => void }) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onBack()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onBack])
  if (page.kind !== 'paint' && page.kind !== 'token') return null
  if (page.kind === 'token') {
    return (
      <div className="flex flex-col gap-2 px-2 pb-2 pt-1 text-[12px]">
        <InspectorBack title={page.title} onBack={onBack} />
        <TokenMenu tokens={page.tokens} selected={page.selected} allowNone onPick={(name) => { page.onPick(name); onBack() }} />
      </div>
    )
  }
  return (
    <div className="flex flex-col gap-2 px-2 pb-2 pt-1 text-[12px]">
      <InspectorBack title={page.title} onBack={onBack} />
      <DesignPaintEditor
        paint={page.paint}
        fallback={page.fallback}
        tokens={page.tokens}
        allowImage={page.allowImage}
        allowGradient={page.allowGradient}
        box={page.box}
        natural={page.natural}
        stroke={page.stroke}
        onChange={page.onChange}
        onStoreImage={page.onStoreImage}
      />
    </div>
  )
}

function numberField(label: string, aria: string, value: number, onChange: (value: number) => void) {
  return <PaintNumberField label={label} ariaLabel={aria} value={value} onChange={onChange} />
}

function PaintNumberField({ label, ariaLabel, value, onChange, suffix }: { label: string; ariaLabel: string; value: number; onChange: (value: number) => void; suffix?: string }) {
  const { inputProps } = useDraftNumber({ value, onChange })
  return (
    <label className="flex h-8 min-w-0 items-center gap-2 rounded-lg bg-koma-bg px-2 focus-within:outline focus-within:outline-1 focus-within:outline-koma-accent">
      <span className="w-14 flex-none truncate text-[11px] text-koma-dim">{label}</span>
      <input {...inputProps} aria-label={ariaLabel} className="h-6 min-w-0 flex-1 bg-transparent text-[12px] text-koma-fg outline-none" />
      {suffix ? <span className="flex-none text-[11px] text-koma-dim">{suffix}</span> : null}
    </label>
  )
}

function StrokePaintFields({ paint, onChange }: { paint: DesignPaint; onChange: (paint: DesignPaint) => void }) {
  const [sides, setSides] = useState(paint.top != null || paint.right != null || paint.bottom != null || paint.left != null)
  const patch = (next: Partial<DesignPaint>) => onChange({ ...paint, ...next })
  return (
    <div className="flex flex-col gap-2">
      {numberField('Width', 'Stroke width', paint.width ?? 1, (width) => patch({ width: width > 0 ? width : undefined }))}
      <LabeledControl label="Align">
        <KomaSelect
          aria-label="Align"
          value={paint.align ?? 'center'}
          onChange={(event) => {
            const align = event.target.value
            if (align !== 'inside' && align !== 'center' && align !== 'outside') return
            patch({ align: align === 'center' ? undefined : align })
          }}
          className="h-7 w-full px-1.5 text-[12px]"
        >
          <option value="inside">Inside</option>
          <option value="center">Center</option>
          <option value="outside">Outside</option>
        </KomaSelect>
      </LabeledControl>
      <div className="grid grid-cols-2 gap-1">
        {numberField('Dash', 'Dash', paint.dash ?? 0, (dash) => patch({ dash: dash > 0 ? dash : undefined }))}
        {numberField('Gap', 'Gap', paint.gap ?? 0, (gap) => patch({ gap: gap > 0 ? gap : undefined }))}
      </div>
      <button
        type="button"
        aria-pressed={sides}
        className={`h-8 rounded-lg text-[12px] ${sides ? 'bg-koma-hover text-koma-fg' : 'bg-koma-bg text-koma-dim hover:text-koma-fg'}`}
        onClick={() => {
          setSides((current) => !current)
          if (sides) patch({ top: undefined, right: undefined, bottom: undefined, left: undefined })
        }}
      >
        {sides ? 'One width' : 'Per-side widths'}
      </button>
      {sides ? (
        <div className="grid grid-cols-2 gap-1">
          {numberField('Top', 'Stroke top', paint.top ?? paint.width ?? 1, (top) => patch({ top }))}
          {numberField('Right', 'Stroke right', paint.right ?? paint.width ?? 1, (right) => patch({ right }))}
          {numberField('Bottom', 'Stroke bottom', paint.bottom ?? paint.width ?? 1, (bottom) => patch({ bottom }))}
          {numberField('Left', 'Stroke left', paint.left ?? paint.width ?? 1, (left) => patch({ left }))}
        </div>
      ) : null}
      <LabeledControl label="Cap">
        <KomaSelect aria-label="Cap" value={paint.capStart ?? 'none'} onChange={(event) => patch({ capStart: event.target.value === 'none' ? undefined : event.target.value as DesignPaint['capStart'] })} className="h-7 w-full px-1.5 text-[12px]">
          <option value="none">None</option>
          <option value="round">Round</option>
          <option value="square">Square</option>
        </KomaSelect>
      </LabeledControl>
      <LabeledControl label="End cap">
        <KomaSelect aria-label="End cap" value={paint.capEnd ?? 'none'} onChange={(event) => patch({ capEnd: event.target.value === 'none' ? undefined : event.target.value as DesignPaint['capEnd'] })} className="h-7 w-full px-1.5 text-[12px]">
          <option value="none">None</option>
          <option value="round">Round</option>
          <option value="square">Square</option>
        </KomaSelect>
      </LabeledControl>
      <LabeledControl label="Start">
        <KomaSelect aria-label="Start marker" value={paint.markerStart ?? 'none'} onChange={(event) => patch({ markerStart: event.target.value === 'none' ? undefined : event.target.value as DesignPaint['markerStart'] })} className="h-7 w-full px-1.5 text-[12px]">
          <option value="none">None</option>
          <option value="arrow">Arrow</option>
          <option value="dot">Dot</option>
        </KomaSelect>
      </LabeledControl>
      <LabeledControl label="End">
        <KomaSelect aria-label="End marker" value={paint.markerEnd ?? 'none'} onChange={(event) => patch({ markerEnd: event.target.value === 'none' ? undefined : event.target.value as DesignPaint['markerEnd'] })} className="h-7 w-full px-1.5 text-[12px]">
          <option value="none">None</option>
          <option value="arrow">Arrow</option>
          <option value="dot">Dot</option>
        </KomaSelect>
      </LabeledControl>
      <LabeledControl label="Join">
        <KomaSelect aria-label="Join" value={paint.join ?? 'miter'} onChange={(event) => patch({ join: event.target.value === 'miter' ? undefined : event.target.value as DesignPaint['join'] })} className="h-7 w-full px-1.5 text-[12px]">
          <option value="miter">Miter</option>
          <option value="round">Round</option>
          <option value="bevel">Bevel</option>
        </KomaSelect>
      </LabeledControl>
    </div>
  )
}

export function DesignPaintEditor({
  paint,
  fallback,
  tokens,
  allowGradient = true,
  allowImage = true,
  box,
  natural,
  stroke = false,
  onChange,
  onStoreImage,
}: {
  paint: DesignPaint
  fallback: string
  tokens?: DesignToken[]
  allowGradient?: boolean
  allowImage?: boolean
  box?: { w: number; h: number }
  natural?: { w: number; h: number } | null
  stroke?: boolean
  onChange: (paint: DesignPaint) => void
  onStoreImage?: (hash: string, bytes: Uint8Array, mime: string) => void
}) {
  const fileRef = useRef<HTMLInputElement>(null)
  const [editing, setEditing] = useState(0)
  const [hexDraft, setHexDraft] = useState<string | null>(null)
  const mode = paint.type === 'gradient' ? 'gradient' : paint.type === 'image' ? 'image' : 'solid'
  useEffect(() => {
    setHexDraft(null)
  }, [mode, editing])
  const stops = paint.stops?.length ? paint.stops : [{ color: '#4f46e5', at: 0 }, { color: '#22d3ee', at: 1 }]
  const stop = stops[Math.min(editing, stops.length - 1)] ?? stops[0]
  const solid = paint.color && paint.color !== 'none' ? paint.color : fallback
  const hex = mode === 'gradient' ? stop?.color ?? fallback : solid
  const picker = colorInputHex(hex, fallback)
  const imageUrl = paint.hash ? designImageUrl(paint.hash) : ''
  const opacity = Math.round((paint.opacity ?? 1) * 100)

  const setSolid = (color: string) => onChange({ ...solidPaint(color), opacity: paint.opacity, visible: paint.visible })
  const setStops = (next: { color: string; at: number }[], kind = paint.kind ?? 'linear') => {
    onChange({ type: 'gradient', kind, stops: next, opacity: paint.opacity, visible: paint.visible })
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
    const size = await imageNaturalSize(bytes, mime)
    rememberImageSize(hash, size.w, size.h)
    onStoreImage?.(hash, bytes, mime)
    const next = { ...imagePaint(hash, paint.scale ?? 'fill'), opacity: paint.opacity, visible: paint.visible }
    onChange(next.scale === 'crop' && box ? ensureImageCrop(next, box, size) : next)
  }
  const applyHex = (raw: string) => {
    const color = parseHex(raw, tokens)
    if (!color) return
    if (mode === 'gradient') setStops(stops.map((item, index) => (index === editing ? { ...item, color } : item)))
    else setSolid(color)
  }

  return (
    <div className="flex flex-col gap-2">
      <LabeledControl label="Type">
        <KomaSelect
          aria-label="Fill type"
          value={mode}
          onChange={(event) => {
            const next = event.target.value
            if (next === 'solid') setSolid(solid.startsWith('#') ? solid : fallback)
            else if (next === 'gradient') onChange({ ...defaultGradient(paint.kind ?? 'linear'), opacity: paint.opacity, visible: paint.visible })
            else if (next === 'image') onChange({ type: 'image', hash: paint.hash, scale: paint.scale ?? 'fill', opacity: paint.opacity, visible: paint.visible })
          }}
          className="h-7 w-full px-1.5 text-[12px]"
        >
          <option value="solid">Solid</option>
          {allowGradient ? <option value="gradient">Gradient</option> : null}
          {allowImage ? <option value="image">Image</option> : null}
        </KomaSelect>
      </LabeledControl>
      {mode === 'image' ? (
        <div className="flex flex-col gap-2">
          <div
            className="flex h-[140px] items-center justify-center overflow-hidden rounded-lg"
            style={{
              backgroundColor: '#2a2d37',
              backgroundImage: 'conic-gradient(#3a3d48 0 25%, #2a2d37 0 50%, #3a3d48 0 75%, #2a2d37 0)',
              backgroundSize: '16px 16px',
            }}
          >
            {imageUrl ? <img src={imageUrl} alt="" className="max-h-full max-w-full object-contain" /> : <span className="text-[11px] text-koma-dim">No image</span>}
          </div>
          <LabeledControl label="Scale">
            <KomaSelect
              aria-label="Image scale"
              value={paint.scale ?? 'fill'}
              onChange={(event) => {
                const scale = event.target.value
                if (scale !== 'fill' && scale !== 'fit' && scale !== 'crop' && scale !== 'tile') return
                if (scale === 'crop') {
                  onChange(box ? ensureImageCrop(paint, box, natural, paint.scale ?? 'fill') : { ...paint, type: 'image' as const, scale })
                  return
                }
                onChange(clearImageCrop({ ...paint, type: 'image' as const, scale }))
              }}
              className="h-7 w-full px-1.5 text-[12px]"
            >
              <option value="fill">Fill</option>
              <option value="fit">Fit</option>
              <option value="crop">Crop</option>
              <option value="tile">Tile</option>
            </KomaSelect>
          </LabeledControl>
          <button
            type="button"
            className="h-8 w-full rounded-lg bg-koma-bg text-[12px] text-koma-fg hover:bg-koma-hover"
            onClick={() => fileRef.current?.click()}
          >
            {paint.hash ? 'Replace image' : 'Choose image'}
          </button>
        </div>
      ) : null}
      {mode === 'gradient' ? (
        <div className="flex flex-col gap-2">
          <LabeledControl label="Style">
            <KomaSelect
              aria-label="Gradient kind"
              value={paint.kind ?? 'linear'}
              onChange={(event) => setStops(stops, event.target.value as DesignGradientKind)}
              className="h-7 w-full px-1.5 text-[12px]"
            >
              {KINDS.map((kind) => (
                <option key={kind.value} value={kind.value}>{kind.label}</option>
              ))}
            </KomaSelect>
          </LabeledControl>
          <GradientRamp stops={stops} selected={editing} onSelect={setEditing} onStops={(next) => setStops(next)} />
        </div>
      ) : null}
      {mode === 'solid' || mode === 'gradient' ? (
        <div className="flex flex-col gap-2">
          <HsvaWell hex={picker} onChange={applyHex} />
          {picker.startsWith('#') ? (
            <div className="flex flex-col gap-1">
              {(['h', 's', 'b'] as const).map((key) => {
                const hsb = hexToHsb(picker)
                const max = key === 'h' ? 360 : 100
                const label = key === 'h' ? 'Hue' : key === 's' ? 'Sat' : 'Bri'
                return (
                  <label key={key} className="flex h-8 items-center gap-2 rounded-lg bg-koma-bg px-2">
                    <span className="w-8 flex-none text-[11px] text-koma-dim">{label}</span>
                    <input
                      type="range"
                      aria-label={label}
                      min={0}
                      max={max}
                      value={Math.round(hsb[key])}
                      onChange={(event) => {
                        const next = { ...hsb, [key]: Number(event.target.value) }
                        applyHex(hsbToHex(next.h, next.s, next.b))
                      }}
                      className="min-w-0 flex-1"
                    />
                  </label>
                )
              })}
            </div>
          ) : null}
          <button
            type="button"
            className="h-8 rounded-lg bg-koma-bg text-[12px] text-koma-fg hover:bg-koma-hover"
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
          <div className="grid grid-cols-[1fr_72px] gap-1">
            <label className="flex h-8 min-w-0 items-center gap-2 rounded-lg bg-koma-bg px-2">
              <span className="flex-none text-[11px] text-koma-dim">Hex</span>
              <input
                aria-label="Hex"
                value={hexDraft ?? (hex.startsWith('#') ? hex.slice(1).toUpperCase() : hex)}
                onFocus={(event) => event.currentTarget.select()}
                onChange={(event) => {
                  const raw = event.target.value
                  setHexDraft(raw)
                  const color = parseHex(raw, tokens)
                  if (color) applyHex(color)
                }}
                onBlur={() => {
                  if (hexDraft != null) applyHex(hexDraft)
                  setHexDraft(null)
                }}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') {
                    event.preventDefault()
                    if (hexDraft != null) applyHex(hexDraft)
                    event.currentTarget.blur()
                  }
                  if (event.key === 'Escape') {
                    setHexDraft(null)
                    event.currentTarget.blur()
                  }
                }}
                className="h-6 min-w-0 flex-1 bg-transparent text-[12px] uppercase text-koma-fg outline-none"
              />
            </label>
            <PaintNumberField label="%" ariaLabel="Opacity" value={opacity} onChange={setOpacity} />
          </div>
          {picker.startsWith('#') ? (
            <div className="grid grid-cols-2 gap-1">
              {(['r', 'g', 'b'] as const).map((key) => {
                const rgba = hexToRgba(picker)
                return (
                  <PaintNumberField
                    key={key}
                    label={key.toUpperCase()}
                    ariaLabel={key.toUpperCase()}
                    value={rgba[key]}
                    onChange={(value) => applyHex(rgbaToHex(key === 'r' ? value : rgba.r, key === 'g' ? value : rgba.g, key === 'b' ? value : rgba.b))}
                  />
                )
              })}
              <PaintNumberField label="A" ariaLabel="Alpha" value={opacity} onChange={setOpacity} />
            </div>
          ) : null}
          {tokens?.length ? (
            <div className="flex flex-col gap-1">
              <span className="px-0.5 text-[11px] text-koma-dim">Tokens</span>
              <TokenMenu tokens={tokens} selected={paint.color} onPick={(name) => { if (name) applyHex(name) }} />
            </div>
          ) : null}
        </div>
      ) : null}
      {mode === 'image' ? (
        <PaintNumberField label="Opacity" ariaLabel="Opacity" value={opacity} onChange={setOpacity} suffix="%" />
      ) : null}
      {stroke ? <StrokePaintFields paint={paint} onChange={onChange} /> : null}
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
    </div>
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
  if (paint.type === 'image') return paint.hash ? 'Image' : 'Choose image'
  if (paint.type === 'gradient') {
    if (paint.kind === 'radial') return 'Radial'
    if (paint.kind === 'angular') return 'Angular'
    if (paint.kind === 'diamond') return 'Diamond'
    return 'Linear'
  }
  if (paint.color && paint.color !== 'none' && !paint.color.startsWith('#')) return paint.color
  return ''
}

