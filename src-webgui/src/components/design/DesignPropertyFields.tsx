import { useEffect, useRef, useState, type ReactNode } from 'react'
import {
  Ban,
  Blend,
  ChevronRight,
  Circle,
  Component,
  Contrast,
  Crop,
  Droplet,
  Eye,
  EyeOff,
  Frame,
  Grid3x3,
  Group,
  Image,
  Maximize2,
  Minus,
  Scaling,
  Square,
  Spline,
  Type,
} from 'lucide-react'
import {
  defaultGradient,
  designLayerName,
  effectiveInstanceChild,
  firstVisiblePaint,
  mergeDesignOverride,
  nodePaints,
  pickVariant,
  setNodePaints,
  setNodeSolid,
  solidPaint,
  type DesignDoc,
  type DesignGradientKind,
  type DesignImageScale,
  type DesignNode,
  type DesignPaint,
  type DesignPenPoint,
} from '../../lib/design'
import { DESIGN_PATCH_DEBOUNCE_MS, SELECTION, SHAPE_FILL, type PenDraft } from './tabShared'

function instanceDescendants(node: DesignNode, into: DesignNode[]) {
  for (const child of node.children ?? []) {
    into.push(child)
    instanceDescendants(child, into)
  }
}

export function InstanceOverrides({
  doc,
  node,
  onPatch,
  onTypeFocus,
  onTypeBlur,
  onPickTarget,
}: {
  doc: DesignDoc
  node: DesignNode
  onPatch: (fn: (node: DesignNode) => DesignNode) => void
  onTypeFocus: () => void
  onTypeBlur: () => void
  onPickTarget?: (childId: string | null) => void
}) {
  if (node.kind !== 'instance' || !node.component) return null
  const component = doc.components.find((item) => item.id === node.component)
  const variant = component ? pickVariant(component, node.variant) : null
  const rows: DesignNode[] = []
  if (variant) instanceDescendants(variant.node, rows)
  if (!rows.length) return null
  const commitFill = (id: string, raw: string) => {
    const value = raw.trim()
    if (!value) onPatch((current) => mergeDesignOverride(current, id, { fill: null }))
    else if (value === 'none' || /^#[0-9a-fA-F]{6}$/.test(value) || /^[a-zA-Z][\w.-]*$/.test(value)) onPatch((current) => mergeDesignOverride(current, id, { fill: value }))
  }
  return (
    <Section title="Overrides">
      {rows.map((child) => {
        const effective = effectiveInstanceChild(doc, node, child.id)
        if (!effective) return null
        const name = designLayerName(child)
        const hidden = !effective.visible
        return (
          <div key={child.id} className="flex flex-col gap-1">
            <button
              type="button"
              className="flex items-center gap-1 rounded text-left hover:bg-koma-hover"
              onClick={() => onPickTarget?.(child.id)}
            >
              <span
                role="button"
                tabIndex={-1}
                aria-label={hidden ? `Show ${name}` : `Hide ${name}`}
                onClick={(event) => {
                  event.stopPropagation()
                  const nextVisible = hidden
                  onPatch((current) => mergeDesignOverride(current, child.id, { visible: nextVisible ? true : false }))
                }}
                className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-dim"
              >
                {hidden ? <EyeOff size={14} /> : <Eye size={14} />}
              </span>
              <span className="min-w-0 flex-1 truncate text-koma-fg">{name}</span>
            </button>
            {child.kind === 'text' ? (
              <input
                aria-label={`${name} text`}
                value={effective.text}
                onFocus={() => {
                  onPickTarget?.(child.id)
                  onTypeFocus()
                }}
                onBlur={onTypeBlur}
                onChange={(event) => onPatch((current) => mergeDesignOverride(current, child.id, { text: event.target.value }))}
                className="h-7 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
              />
            ) : null}
            <input
              aria-label={`${name} fill`}
              value={effective.fill && effective.fill !== 'none' ? effective.fill : ''}
              placeholder="Fill"
              onFocus={() => onPickTarget?.(child.id)}
              onChange={(event) => commitFill(child.id, event.target.value)}
              className="h-7 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
            />
          </div>
        )
      })}
    </Section>
  )
}

export function Section({ title, children }: { title: string; children: ReactNode }) {
  const [open, setOpen] = useState(true)
  return (
    <section className="flex flex-col border-t border-koma-border">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        className="flex h-[22px] items-center gap-1 bg-koma-head px-2 text-left text-[11px] font-semibold uppercase tracking-wide text-koma-fg opacity-75 hover:bg-koma-hover hover:opacity-100"
      >
        <ChevronRight size={14} strokeWidth={2} className={`transition-transform ${open ? 'rotate-90' : ''}`} />
        <span className="truncate">{title}</span>
      </button>
      {open ? <div className="flex flex-col gap-2 px-2 py-2">{children}</div> : null}
    </section>
  )
}

function formatBareNumber(value: number): string {
  if (!Number.isFinite(value)) return '0'
  const rounded = Math.round(value * 100) / 100
  return String(rounded)
}

function isBareNumberDraft(raw: string): boolean {
  return raw === '' || /^-?\d*\.?\d*$/.test(raw)
}

function parseBareNumber(raw: string): number {
  const trimmed = raw.trim()
  if (trimmed === '' || trimmed === '-' || trimmed === '.') return 0
  const next = Number(trimmed)
  return Number.isFinite(next) ? next : 0
}

export function GeomField({ label, ariaLabel, value, mixed, suffix, onChange }: { label: string; ariaLabel?: string; value: number; mixed?: boolean; suffix?: string; onChange: (value: number) => void }) {
  const [draft, setDraft] = useState<string | null>(null)
  const focused = useRef(false)
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  useEffect(() => {
    if (!focused.current) setDraft(null)
  }, [value, mixed])
  useEffect(() => () => {
    if (debounceRef.current) clearTimeout(debounceRef.current)
  }, [])
  const commitRaw = (raw: string) => {
    onChange(parseBareNumber(raw))
  }
  const shown = mixed ? '' : draft ?? formatBareNumber(value)
  return (
    <label className="flex h-7 items-center gap-1 rounded border border-koma-border bg-koma-bg px-1.5">
      <span className="flex-none text-[11px] text-koma-dim">{label}</span>
      <input
        type="text"
        inputMode="decimal"
        value={shown}
        placeholder={mixed ? 'Mixed' : undefined}
        aria-label={ariaLabel ?? label}
        onFocus={() => {
          focused.current = true
          if (!mixed) setDraft(formatBareNumber(value))
        }}
        onChange={(event) => {
          if (mixed) return
          const raw = event.target.value
          if (!isBareNumberDraft(raw)) return
          setDraft(raw)
          if (debounceRef.current) clearTimeout(debounceRef.current)
          debounceRef.current = setTimeout(() => {
            debounceRef.current = null
            if (focused.current) commitRaw(raw)
          }, DESIGN_PATCH_DEBOUNCE_MS)
        }}
        onBlur={() => {
          focused.current = false
          if (mixed) return
          if (debounceRef.current) {
            clearTimeout(debounceRef.current)
            debounceRef.current = null
          }
          commitRaw(draft ?? formatBareNumber(value))
          setDraft(null)
        }}
        className="h-6 min-w-0 flex-1 bg-transparent text-[12px] text-koma-fg outline-none"
      />
      {suffix ? <span className="flex-none text-[11px] text-koma-dim">{suffix}</span> : null}
    </label>
  )
}

export function AlignButton({ label, pressed, onClick, children }: { label: string; pressed?: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button type="button" aria-label={label} title={label} aria-pressed={pressed} onClick={onClick} className={`flex h-7 w-7 flex-none items-center justify-center rounded ${pressed ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`}>
      {children}
    </button>
  )
}

export function KindMark({ kind }: { kind: DesignNode['kind'] }) {
  const props = { size: 14, strokeWidth: 2 }
  if (kind === 'frame') return <Frame {...props} />
  if (kind === 'group') return <Group {...props} />
  if (kind === 'ellipse') return <Circle {...props} />
  if (kind === 'line') return <Minus {...props} />
  if (kind === 'vector') return <Spline {...props} />
  if (kind === 'text') return <Type {...props} />
  if (kind === 'instance') return <Component {...props} />
  return <Square {...props} />
}

export function SizeMode({ label, value, mixed, onChange }: { label: string; value: string; mixed?: boolean; onChange: (mode: 'fixed' | 'hug' | 'fill') => void }) {
  return (
    <Choices
      label={label}
      value={mixed ? '' : value}
      mixed={mixed}
      grow={false}
      options={[
        { value: 'fixed', label: 'Fixed', icon: <Square size={13} /> },
        { value: 'hug', label: 'Hug', icon: <MinimizeIcon /> },
        { value: 'fill', label: 'Fill', icon: <Maximize2 size={13} /> },
      ]}
      onChange={(mode) => {
        if (mode === 'fixed' || mode === 'hug' || mode === 'fill') onChange(mode)
      }}
    />
  )
}

function MinimizeIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden>
      <path d="M8 3v3a2 2 0 0 1-2 2H3" />
      <path d="M21 8h-3a2 2 0 0 1-2-2V3" />
      <path d="M3 16h3a2 2 0 0 1 2 2v3" />
      <path d="M16 21v-3a2 2 0 0 1 2-2h3" />
    </svg>
  )
}

export function StrokeAlignIcon({ mode }: { mode: 'inside' | 'center' | 'outside' }) {
  const inset = mode === 'inside' ? 5 : mode === 'outside' ? 1.5 : 3.5
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden>
      <rect x={inset} y={inset} width={14 - inset * 2} height={14 - inset * 2} fill="currentColor" opacity="0.22" />
      <rect x="3.5" y="3.5" width="7" height="7" fill="none" stroke="currentColor" strokeWidth={mode === 'center' ? 2 : 1.5} />
    </svg>
  )
}

export function RadiusField({ value, mixed, resolved, tokens, onChange }: { value: number | string | undefined; mixed?: boolean; resolved?: string; tokens: { name: string }[]; onChange: (radius: number | string | null) => void }) {
  const [open, setOpen] = useState(false)
  const [draft, setDraft] = useState<string | null>(null)
  const focused = useRef(false)
  const token = !mixed && typeof value === 'string' ? value : ''
  const numericShown =
    typeof value === 'number' ? formatBareNumber(value) : resolved && !token ? resolved : '0'
  useEffect(() => {
    if (!focused.current) setDraft(null)
  }, [value, mixed, token, resolved])
  const commitRadius = (raw: string) => {
    const trimmed = raw.trim()
    const named = tokens.find((item) => item.name === trimmed)
    if (named) {
      onChange(named.name)
      return
    }
    const radius = parseBareNumber(raw)
    if (radius <= 0) onChange(null)
    else onChange(radius)
  }
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <label className="flex h-7 items-center gap-1 rounded border border-koma-border bg-koma-bg px-1.5">
        <span className="flex-none text-[11px] text-koma-dim">R</span>
        {tokens.length ? (
          <button
            type="button"
            aria-label="Corner radius token"
            aria-expanded={open}
            aria-pressed={!!token}
            onClick={() => setOpen((current) => !current)}
            className={`h-4 w-4 flex-none rounded-full border border-koma-border ${token ? 'bg-koma-accent' : 'bg-transparent'}`}
          />
        ) : null}
        <input
          type="text"
          inputMode={token ? 'text' : 'decimal'}
          aria-label="Corner radius"
          value={mixed ? '' : token || draft || numericShown}
          placeholder={mixed ? 'Mixed' : undefined}
          onFocus={() => {
            focused.current = true
            if (!mixed && !token) setDraft(typeof value === 'number' ? formatBareNumber(value) : numericShown)
          }}
          onChange={(event) => {
            if (mixed) return
            const raw = event.target.value
            if (token) {
              setDraft(raw)
              return
            }
            if (isBareNumberDraft(raw)) setDraft(raw)
          }}
          onBlur={() => {
            focused.current = false
            if (mixed) return
            commitRadius(draft ?? (token || numericShown))
            setDraft(null)
          }}
          className="h-6 min-w-0 flex-1 bg-transparent text-[12px] text-koma-fg outline-none"
        />
      </label>
      {open ? (
        <div className="flex flex-col gap-0.5 rounded border border-koma-border p-1">
          <button
            type="button"
            onClick={() => {
              const radius = Number(resolved)
              onChange(Number.isFinite(radius) && radius > 0 ? radius : null)
              setOpen(false)
            }}
            className="h-6 rounded px-1 text-left text-[12px] text-koma-dim hover:bg-koma-hover"
          >
            Number
          </button>
          {tokens.map((item) => (
            <button
              key={item.name}
              type="button"
              aria-pressed={token === item.name}
              onClick={() => {
                onChange(item.name)
                setOpen(false)
              }}
              className={`h-6 rounded px-1 text-left text-[12px] ${token === item.name ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover'}`}
            >
              {item.name}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  )
}

export function containerPaint(node: DesignNode, field: 'fill' | 'stroke', chrome: string): string {
  if (node.kind === 'group' && node[field] == null) return 'none'
  if (node.kind === 'frame' && field === 'fill' && node.fill == null) return '#ffffff'
  if (node.kind === 'frame' && field === 'stroke' && node.stroke == null) return 'none'
  if ((node.kind === 'rect' || node.kind === 'ellipse') && field === 'fill' && node.fill == null) return SHAPE_FILL
  if ((node.kind === 'rect' || node.kind === 'ellipse') && field === 'stroke' && node.stroke == null) return 'none'
  if ((node.kind === 'line' || node.kind === 'vector') && field === 'stroke' && node.stroke == null) return '#1c1c1c'
  return chrome
}

function penCurve(points: DesignPenPoint[], closed: boolean): string {
  if (!points.length) return ''
  let path = `M ${points[0].x} ${points[0].y}`
  const count = closed ? points.length : points.length - 1
  for (let index = 0; index < count; index++) {
    const start = points[index]
    const end = points[(index + 1) % points.length]
    path += ` C ${start.x + start.outgoing.x} ${start.y + start.outgoing.y} ${end.x + end.incoming.x} ${end.y + end.incoming.y} ${end.x} ${end.y}`
  }
  if (closed) path += ' Z'
  return path
}

export function PenOverlay({ draft, hover, zoom }: { draft: PenDraft; hover: { x: number; y: number } | null; zoom: number }) {
  const unit = 1 / Math.max(zoom, 0.25)
  const points = draft.points
  const last = points[points.length - 1]
  const first = points[0]
  const close = hover && first && points.length >= 3 && Math.hypot(first.x - hover.x, first.y - hover.y) <= 8 * unit
  const rubber = hover && last && !close ? `M ${last.x} ${last.y} C ${last.x + last.outgoing.x} ${last.y + last.outgoing.y} ${hover.x} ${hover.y} ${hover.x} ${hover.y}` : ''
  return (
    <svg className="pointer-events-none absolute overflow-visible" width={1} height={1}>
      <path d={penCurve(points, false)} fill="none" stroke="#1c1c1c" strokeWidth={2} />
      {rubber ? <path d={rubber} fill="none" stroke={SELECTION} strokeWidth={1.25 * unit} /> : null}
      {first && points.length >= 3 ? <circle cx={first.x} cy={first.y} r={close ? 7 * unit : 5 * unit} fill={close ? SELECTION : 'transparent'} stroke={SELECTION} strokeWidth={unit} /> : null}
      {points.map((point, index) => {
        const showOut = point.outgoing.x !== 0 || point.outgoing.y !== 0
        const showIn = point.incoming.x !== 0 || point.incoming.y !== 0
        return (
          <g key={`${draft.id}-${index}`}>
            {showOut ? <line x1={point.x} y1={point.y} x2={point.x + point.outgoing.x} y2={point.y + point.outgoing.y} stroke={SELECTION} strokeWidth={unit} /> : null}
            {showIn ? <line x1={point.x} y1={point.y} x2={point.x + point.incoming.x} y2={point.y + point.incoming.y} stroke={SELECTION} strokeWidth={unit} /> : null}
            {showOut ? <circle cx={point.x + point.outgoing.x} cy={point.y + point.outgoing.y} r={3 * unit} fill="#ffffff" stroke={SELECTION} strokeWidth={unit} /> : null}
            {showIn ? <circle cx={point.x + point.incoming.x} cy={point.y + point.incoming.y} r={3 * unit} fill="#ffffff" stroke={SELECTION} strokeWidth={unit} /> : null}
            <rect x={point.x - 3.5 * unit} y={point.y - 3.5 * unit} width={7 * unit} height={7 * unit} fill="#ffffff" stroke={SELECTION} strokeWidth={unit} />
          </g>
        )
      })}
    </svg>
  )
}

export function NumberField({ label, value, onChange }: { label: string; value: number; onChange: (value: number) => void }) {
  const [draft, setDraft] = useState<string | null>(null)
  const focused = useRef(false)
  useEffect(() => {
    if (!focused.current) setDraft(null)
  }, [value])
  return (
    <label className="flex flex-col gap-1">
      <span className="text-koma-dim">{label}</span>
      <input
        type="text"
        inputMode="decimal"
        value={draft ?? formatBareNumber(value)}
        onFocus={() => {
          focused.current = true
          setDraft(formatBareNumber(value))
        }}
        onChange={(event) => {
          const raw = event.target.value
          if (isBareNumberDraft(raw)) setDraft(raw)
        }}
        onBlur={() => {
          focused.current = false
          onChange(parseBareNumber(draft ?? formatBareNumber(value)))
          setDraft(null)
        }}
        className="h-7 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
      />
    </label>
  )
}

function colorInputHex(value: string, fallback: string): string {
  const source = /^#[0-9a-fA-F]{3,8}$/.test(value) ? value : fallback
  const body = source.slice(1)
  if (/^[0-9a-fA-F]{6}$/.test(body) || /^[0-9a-fA-F]{8}$/.test(body)) return `#${body.slice(0, 6).toLowerCase()}`
  if (/^[0-9a-fA-F]{3}$/.test(body)) return `#${body[0]}${body[0]}${body[1]}${body[1]}${body[2]}${body[2]}`.toLowerCase()
  return '#000000'
}

export function PaintRow({ label, value, mixed, fallback, resolved, tokens, weight, onWeight, onChange }: { label: string; value: string; mixed?: boolean; fallback: string; resolved?: string; tokens?: { name: string }[]; weight?: { value: number; mixed?: boolean }; onWeight?: (value: number) => void; onChange: (next: string | null) => void }) {
  const [open, setOpen] = useState(false)
  const [draft, setDraft] = useState<string | null>(null)
  const on = !mixed && value !== 'none'
  const hex = value.startsWith('#') ? value : resolved?.startsWith('#') ? resolved : fallback
  const picker = colorInputHex(hex, fallback)
  const swatch = value.startsWith('#') ? value : resolved?.startsWith('#') ? resolved : on ? 'var(--color-koma-panel)' : 'transparent'
  const shown = mixed ? '' : value === 'none' ? '' : value
  const commitText = (raw: string) => {
    const trimmed = raw.trim()
    setDraft(null)
    if (!trimmed || trimmed.toLowerCase() === 'none') {
      onChange('none')
      return
    }
    const named = tokens?.find((token) => token.name === trimmed)
    if (named) {
      onChange(named.name)
      return
    }
    const body = trimmed.startsWith('#') ? trimmed.slice(1) : trimmed
    if (/^[0-9a-fA-F]{3}$|^[0-9a-fA-F]{6}$|^[0-9a-fA-F]{8}$/.test(body)) onChange(`#${body.toLowerCase()}`)
  }
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-1">
        {tokens?.length ? (
          <button
            type="button"
            aria-label={`${label} swatch`}
            aria-expanded={open}
            onClick={() => setOpen((current) => !current)}
            className="h-7 w-7 flex-none rounded border border-koma-border"
            style={{ background: swatch }}
          />
        ) : (
          <label className="relative h-7 w-7 flex-none overflow-hidden rounded border border-koma-border">
            <span className="absolute inset-0" style={{ background: swatch }} />
            <input
              type="color"
              aria-label={`${label} color`}
              value={picker}
              onChange={(event) => onChange(event.target.value.toLowerCase())}
              className="absolute inset-0 cursor-pointer opacity-0"
            />
          </label>
        )}
        <input
          aria-label={`${label} hex`}
          value={draft ?? shown}
          placeholder={mixed ? 'Mixed' : value === 'none' ? 'None' : '#hex'}
          onChange={(event) => {
            const raw = event.target.value
            const trimmed = raw.trim()
            const named = tokens?.find((token) => token.name === trimmed)
            const body = trimmed.startsWith('#') ? trimmed.slice(1) : trimmed
            if (named) {
              setDraft(null)
              onChange(named.name)
              return
            }
            if (/^[0-9a-fA-F]{6}$|^[0-9a-fA-F]{8}$/.test(body)) {
              setDraft(null)
              onChange(`#${body.toLowerCase()}`)
              return
            }
            setDraft(raw)
          }}
          onBlur={() => {
            if (draft != null) commitText(draft)
          }}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && draft != null) commitText(draft)
          }}
          className="h-7 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-1.5 text-[12px] text-koma-fg outline-none"
        />
        <button
          type="button"
          aria-label={`${label} visibility`}
          aria-pressed={on}
          title={mixed ? 'Mixed' : on ? 'On' : 'Off'}
          onClick={() => onChange(mixed || on ? 'none' : null)}
          className={`flex h-7 w-7 flex-none items-center justify-center rounded ${on ? 'text-koma-fg' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`}
        >
          {on ? <Eye size={14} /> : <EyeOff size={14} />}
        </button>
        {weight && onWeight ? (
          <div className="w-16 flex-none">
            <GeomField label="W" ariaLabel="Weight" value={weight.value} mixed={weight.mixed} onChange={onWeight} />
          </div>
        ) : null}
      </div>
      {open && tokens?.length ? (
        <div className="flex flex-col gap-1 rounded border border-koma-border p-1">
          <input
            type="color"
            aria-label={`${label} color`}
            value={picker}
            onChange={(event) => onChange(event.target.value.toLowerCase())}
            className="h-7 w-full cursor-pointer bg-transparent"
          />
          {tokens.map((token) => (
            <button
              key={token.name}
              type="button"
              aria-pressed={value === token.name}
              onClick={() => {
                onChange(token.name)
                setOpen(false)
              }}
              className={`h-6 rounded px-1 text-left text-[12px] ${value === token.name ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover'}`}
            >
              {token.name}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  )
}

export function Choices<T extends string>({
  label,
  value,
  mixed,
  options,
  onChange,
  grow = true,
}: {
  label: string
  value: T | ''
  mixed?: boolean
  options: { value: T; label: string; icon?: ReactNode }[]
  onChange: (value: T) => void
  grow?: boolean
}) {
  const iconic = options.every((option) => option.icon)
  return (
    <div className={`flex flex-col gap-1 ${grow ? 'min-w-0 flex-1' : 'flex-none'}`} role="group" aria-label={mixed ? `${label} · Mixed` : label}>
      <div className={`flex ${iconic ? 'h-7 rounded border border-koma-border bg-koma-bg p-0.5' : 'flex-wrap gap-0.5'} ${grow ? 'min-w-0' : ''}`}>
        {options.map((option) => {
          const pressed = !mixed && option.value === value
          return (
            <button
              key={option.value}
              type="button"
              title={option.label}
              aria-label={option.label}
              aria-pressed={pressed}
              onClick={() => onChange(option.value)}
              className={
                iconic
                  ? `flex h-6 items-center justify-center rounded ${grow ? 'min-w-0 flex-1' : 'w-6 flex-none'} ${pressed ? 'bg-koma-hover text-koma-fg' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`
                  : `h-7 min-w-0 flex-1 rounded px-1 text-[12px] ${pressed ? 'bg-koma-hover text-koma-fg' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`
              }
            >
              {option.icon ?? option.label}
            </button>
          )
        })}
      </div>
    </div>
  )
}

export function FillEditor({
  label,
  doc,
  node,
  field,
  mixed,
  tokens,
  onChange,
  onPickImage,
}: {
  label: string
  doc: DesignDoc
  node: DesignNode
  field: 'fill' | 'stroke'
  mixed?: boolean
  tokens?: { name: string }[]
  onChange: (node: DesignNode) => void
  onPickImage?: () => void
}) {
  const paints = nodePaints(node, field)
  const current = firstVisiblePaint(paints)
  const kind = current?.type === 'gradient' ? 'gradient' : current?.type === 'image' ? 'image' : current && paintAliasSafe(current) === 'none' ? 'none' : 'solid'
  const solid = kind === 'solid' ? current?.color ?? (field === 'fill' ? SHAPE_FILL : '#1c1c1c') : field === 'fill' ? SHAPE_FILL : '#1c1c1c'
  return (
    <div className="flex flex-col gap-1">
      <Choices
        label={label}
        value={kind}
        mixed={mixed}
        options={[
          { value: 'solid', label: 'Solid', icon: <Droplet size={13} /> },
          { value: 'gradient', label: 'Gradient', icon: <Blend size={13} /> },
          { value: 'image', label: 'Image', icon: <Image size={13} /> },
          { value: 'none', label: 'None', icon: <Ban size={13} /> },
        ]}
        onChange={(next) => {
          if (next === 'none') onChange(setNodeSolid(node, field, 'none'))
          else if (next === 'solid') onChange(setNodeSolid(node, field, solid.startsWith('#') || tokens?.some((token) => token.name === solid) ? solid : SHAPE_FILL))
          else if (next === 'gradient') onChange(setNodePaints(node, field, [defaultGradient(current?.kind ?? 'linear')]))
          else if (next === 'image') onPickImage?.()
        }}
      />
      {kind === 'solid' ? (
        <PaintRow
          label={label}
          value={solid}
          mixed={mixed}
          fallback={field === 'fill' ? SHAPE_FILL : '#1c1c1c'}
          resolved={solid.startsWith('#') ? solid : undefined}
          tokens={tokens}
          onChange={(next) => onChange(setNodeSolid(node, field, next))}
        />
      ) : null}
      {kind === 'gradient' && current ? (
        <GradientEditor
          paint={current}
          tokens={tokens}
          onChange={(paint) => onChange(setNodePaints(node, field, [paint]))}
        />
      ) : null}
      {kind === 'image' && current ? (
        <div className="flex flex-col gap-1">
          <button type="button" title="Replace image" aria-label="Replace image" className="flex h-7 items-center justify-center rounded border border-koma-border text-koma-dim hover:bg-koma-hover hover:text-koma-fg" onClick={() => onPickImage?.()}>
            <Image size={13} />
          </button>
          <Choices
            label="Scale"
            value={current.scale ?? 'fill'}
            options={[
              { value: 'fill', label: 'Fill', icon: <Maximize2 size={13} /> },
              { value: 'fit', label: 'Fit', icon: <MinimizeIcon /> },
              { value: 'crop', label: 'Crop', icon: <Crop size={13} /> },
              { value: 'tile', label: 'Tile', icon: <Grid3x3 size={13} /> },
            ]}
            onChange={(scale: DesignImageScale) => onChange(setNodePaints(node, field, [{ ...current, scale }]))}
          />
        </div>
      ) : null}
    </div>
  )
}

function paintAliasSafe(paint: DesignPaint): string {
  if (paint.visible === false) return 'none'
  if (paint.type === 'solid') return paint.color && paint.color !== 'none' ? paint.color : 'none'
  return ''
}

export function GradientEditor({
  paint,
  tokens,
  onChange,
}: {
  paint: DesignPaint
  tokens?: { name: string }[]
  onChange: (paint: DesignPaint) => void
}) {
  const stops = paint.stops?.length ? paint.stops : [{ color: '#4f46e5', at: 0 }, { color: '#22d3ee', at: 1 }]
  return (
    <div className="flex flex-col gap-1">
      <Choices
        label="Kind"
        value={paint.kind ?? 'linear'}
        options={[
          { value: 'linear', label: 'Linear', icon: <Minus size={13} /> },
          { value: 'radial', label: 'Radial', icon: <Circle size={13} /> },
          { value: 'angular', label: 'Angular', icon: <Contrast size={13} /> },
          { value: 'diamond', label: 'Diamond', icon: <Scaling size={13} /> },
        ]}
        onChange={(kind: DesignGradientKind) => onChange({ ...paint, type: 'gradient', kind })}
      />
      {stops.map((stop, index) => (
        <div key={`${stop.at}-${index}`} className="flex items-center gap-1">
          <PaintRow
            label={`Stop ${index + 1}`}
            value={stop.color}
            fallback="#000000"
            tokens={tokens}
            onChange={(color) => {
              if (!color || color === 'none') return
              const next = stops.map((item, at) => (at === index ? { ...item, color } : item))
              onChange({ ...paint, type: 'gradient', stops: next })
            }}
          />
          <div className="w-14 flex-none">
            <GeomField
              label="%"
              value={Math.round((Number.isFinite(stop.at) ? stop.at : 0) * 100)}
              onChange={(value) => {
                const next = stops.map((item, at) => (at === index ? { ...item, at: Math.max(0, Math.min(1, value / 100)) } : item))
                onChange({ ...paint, type: 'gradient', stops: next })
              }}
            />
          </div>
          {stops.length > 2 ? (
            <button
              type="button"
              aria-label="Remove stop"
              className="h-7 w-7 flex-none rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
              onClick={() => onChange({ ...paint, type: 'gradient', stops: stops.filter((_, at) => at !== index) })}
            >
              <Minus size={12} />
            </button>
          ) : null}
        </div>
      ))}
      <button
        type="button"
        className="h-7 rounded border border-koma-border text-[12px] text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
        onClick={() => onChange({ ...paint, type: 'gradient', stops: [...stops, { color: '#ffffff', at: 1 }] })}
      >
        Add stop
      </button>
    </div>
  )
}
