import { useEffect, useRef, useState, type ReactNode } from 'react'
import {
  Circle,
  Component,
  Eye,
  EyeOff,
  Frame,
  Group,
  Maximize2,
  Minus,
  Square,
  Spline,
  Type,
} from 'lucide-react'
import { AccordionSection } from '../AccordionSection'
import { KomaSelect } from '../KomaSelect'
import { TokenMenu } from './DesignTokenMenu'
import type { DesignToken } from '../../lib/design'
import {
  bindNodeField,
  designLayerName,
  effectiveInstanceChild,
  mergeDesignOverride,
  nodePaints,
  pickVariant,
  reorderNodePaint,
  setNodePaints,
  setNodeSolid,
  solidPaint,
  type DesignDoc,
  type DesignConstraint,
  type DesignNode,
  type DesignPaint,
  type DesignPenPoint,
} from '../../lib/design'
import { DESIGN_PATCH_DEBOUNCE_MS, SELECTION, SHAPE_FILL, type PenDraft } from './tabShared'
import { DesignPaintPopup, paintRowLabel, paintSwatch } from './DesignPaintPopup'

function TokenBindControl({
  tokens,
  bound,
  onBind,
  compact,
}: {
  tokens: DesignToken[]
  bound?: string
  onBind: (token: string | null) => void
  compact?: boolean
}) {
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const onDoc = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', onDoc)
    return () => document.removeEventListener('mousedown', onDoc)
  }, [open])
  return (
    <div ref={rootRef} className="relative flex-none">
      <button
        type="button"
        title={bound ? `Bound to ${bound}` : 'Bind token'}
        aria-label={bound ? `Bound to ${bound}` : 'Bind token'}
        aria-pressed={!!bound}
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
        className={compact
          ? `h-4 w-4 rounded-full border border-koma-border ${bound ? 'bg-koma-accent' : 'bg-transparent'}`
          : `flex h-7 w-7 items-center justify-center rounded ${bound ? 'text-koma-accent' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`}
      >
        {compact ? null : <span className={`h-3 w-3 rounded-full border ${bound ? 'border-koma-accent bg-koma-accent' : 'border-koma-border'}`} />}
      </button>
      {open ? (
        <div className="absolute right-0 z-50 mt-1 w-56 rounded border border-koma-border bg-koma-panel p-1 shadow-lg">
          <TokenMenu
            tokens={tokens}
            selected={bound}
            allowNone
            onPick={(name) => {
              onBind(name)
              setOpen(false)
            }}
          />
        </div>
      ) : null}
    </div>
  )
}

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
            <div className="grid grid-cols-2 gap-1">
              <input
                aria-label={`${name} stroke`}
                value={effective.stroke && effective.stroke !== 'none' ? effective.stroke : ''}
                placeholder="Stroke"
                onFocus={() => onPickTarget?.(child.id)}
                onChange={(event) => {
                  const value = event.target.value.trim()
                  onPatch((current) => mergeDesignOverride(current, child.id, { stroke: value || null }))
                }}
                className="h-7 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
              />
              <input
                aria-label={`${name} color`}
                value={effective.color && effective.color !== 'none' ? effective.color : ''}
                placeholder="Color"
                onFocus={() => onPickTarget?.(child.id)}
                onChange={(event) => onPatch((current) => mergeDesignOverride(current, child.id, { color: event.target.value.trim() || null }))}
                className="h-7 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
              />
              <GeomField label="Op" ariaLabel={`${name} opacity`} suffix="%" value={Math.round(effective.opacity * 100)} onChange={(value) => onPatch((current) => mergeDesignOverride(current, child.id, { opacity: Math.min(100, Math.max(0, value)) / 100 }))} />
              <GeomField label="R" ariaLabel={`${name} radius`} value={typeof effective.radius === 'number' ? effective.radius : 0} onChange={(radius) => onPatch((current) => mergeDesignOverride(current, child.id, { radius: radius > 0 ? radius : null }))} />
              <GeomField label="Sz" ariaLabel={`${name} font size`} value={effective.fontSize} onChange={(fontSize) => onPatch((current) => mergeDesignOverride(current, child.id, { fontSize: fontSize > 0 ? fontSize : null }))} />
              <GeomField label="Sw" ariaLabel={`${name} stroke width`} value={effective.strokeWidth} onChange={(strokeWidth) => onPatch((current) => mergeDesignOverride(current, child.id, { strokeWidth: strokeWidth > 0 ? strokeWidth : null }))} />
              <GeomField label="Rot" ariaLabel={`${name} rotation`} value={effective.rotation} onChange={(rotation) => onPatch((current) => mergeDesignOverride(current, child.id, { rotation: rotation ? rotation : null }))} />
            </div>
          </div>
        )
      })}
    </Section>
  )
}

export function Section({ title, action, children }: { title: string; action?: ReactNode; children?: ReactNode }) {
  const [open, setOpen] = useState(true)
  return (
    <AccordionSection title={title} open={open} onToggle={() => setOpen((value) => !value)} action={action} fill={false}>
      <div className="flex flex-col gap-1 px-2 pt-1">{children}</div>
    </AccordionSection>
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

export function GeomField({ label, ariaLabel, value, mixed, suffix, tokens, bound, onBind, onChange }: { label: string; ariaLabel?: string; value: number; mixed?: boolean; suffix?: string; tokens?: DesignToken[]; bound?: string; onBind?: (token: string | null) => void; onChange: (value: number) => void }) {
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
    <label className="flex h-7 min-w-0 items-center gap-1 rounded border border-koma-border bg-koma-bg px-1.5 focus-within:border-koma-fg/40">
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
      {tokens?.length && onBind ? <TokenBindControl tokens={tokens} bound={bound} onBind={onBind} compact /> : null}
    </label>
  )
}

export function AlignButton({ label, caption, pressed, onClick, children }: { label: string; caption?: string; pressed?: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button type="button" aria-label={label} title={label} aria-pressed={pressed} onClick={onClick} className={`flex h-7 flex-none items-center justify-center gap-1 rounded px-1.5 text-[11px] ${pressed ? 'bg-koma-hover text-koma-fg' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`}>
      {children}
      {caption ? <span>{caption}</span> : null}
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

function toggleConstraint(current: DesignConstraint, edge: 'start' | 'end' | 'center'): DesignConstraint {
  if (edge === 'center') return current === 'center' ? 'scale' : 'center'
  if (edge === 'start') {
    if (current === 'start') return 'scale'
    if (current === 'stretch') return 'end'
    if (current === 'end') return 'stretch'
    return 'start'
  }
  if (current === 'end') return 'scale'
  if (current === 'stretch') return 'start'
  if (current === 'start') return 'stretch'
  return 'end'
}

function ConstraintBar({ active, vertical, label, onClick }: { active: boolean; vertical?: boolean; label: string; onClick: () => void }) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-pressed={active}
      onClick={onClick}
      className="flex h-full w-full items-center justify-center"
    >
      <span className={`rounded-full ${vertical ? 'h-8 w-[3px]' : 'h-[3px] w-8'} ${active ? 'bg-koma-accent' : 'bg-koma-grip'}`} />
    </button>
  )
}

export function ConstraintWidget({
  horizontal,
  vertical,
  onHorizontal,
  onVertical,
}: {
  horizontal: DesignConstraint
  vertical: DesignConstraint
  onHorizontal: (value: DesignConstraint) => void
  onVertical: (value: DesignConstraint) => void
}) {
  return (
    <div
      className="grid h-[108px] w-[108px] flex-none rounded-lg bg-koma-bg"
      style={{ gridTemplate: '"top top top" 24px "left center right" 60px "bottom bottom bottom" 24px / 24px 60px 24px' }}
    >
      <div style={{ gridArea: 'top' }}>
        <ConstraintBar active={vertical === 'start' || vertical === 'stretch'} label="Top" onClick={() => onVertical(toggleConstraint(vertical, 'start'))} />
      </div>
      <div style={{ gridArea: 'left' }}>
        <ConstraintBar vertical active={horizontal === 'start' || horizontal === 'stretch'} label="Left" onClick={() => onHorizontal(toggleConstraint(horizontal, 'start'))} />
      </div>
      <div className="relative rounded-lg bg-koma-panel" style={{ gridArea: 'center' }}>
        <ConstraintBar active={vertical === 'center'} label="Vertical center" onClick={() => onVertical(toggleConstraint(vertical, 'center'))} />
        <div className="absolute inset-0">
          <ConstraintBar vertical active={horizontal === 'center'} label="Horizontal center" onClick={() => onHorizontal(toggleConstraint(horizontal, 'center'))} />
        </div>
      </div>
      <div style={{ gridArea: 'right' }}>
        <ConstraintBar vertical active={horizontal === 'end' || horizontal === 'stretch'} label="Right" onClick={() => onHorizontal(toggleConstraint(horizontal, 'end'))} />
      </div>
      <div style={{ gridArea: 'bottom' }}>
        <ConstraintBar active={vertical === 'end' || vertical === 'stretch'} label="Bottom" onClick={() => onVertical(toggleConstraint(vertical, 'end'))} />
      </div>
    </div>
  )
}

export function RadiusField({ value, mixed, resolved, tokens, onChange }: { value: number | string | undefined; mixed?: boolean; resolved?: string; tokens: DesignToken[]; onChange: (radius: number | string | null) => void }) {
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
      <label className="flex h-7 items-center gap-1 rounded border border-koma-border bg-koma-bg px-1.5 focus-within:border-koma-fg/40">
        <span className="flex-none text-[11px] text-koma-dim">Radius</span>
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
          <TokenMenu
            tokens={tokens}
            selected={token}
            onPick={(name) => {
              if (name) onChange(name)
              setOpen(false)
            }}
          />
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

export function ColorRow({
  doc,
  paint,
  fallback,
  tokens,
  mixed,
  allowImage = true,
  allowGradient = true,
  bound,
  onBind,
  onChange,
  onRemove,
  onStoreImage,
}: {
  doc: DesignDoc
  paint: DesignPaint
  fallback: string
  tokens?: DesignToken[]
  mixed?: boolean
  allowImage?: boolean
  allowGradient?: boolean
  bound?: string
  onBind?: (token: string | null) => void
  onChange: (paint: DesignPaint) => void
  onRemove?: () => void
  onStoreImage?: (hash: string, bytes: Uint8Array, mime: string) => void
}) {
  const [open, setOpen] = useState(false)
  const [hexDraft, setHexDraft] = useState<string | null>(null)
  const swatchRef = useRef<HTMLButtonElement>(null)
  const named = paint.type === 'image' || paint.type === 'gradient'
  const token = paint.type === 'solid' && paint.color && !paint.color.startsWith('#') && paint.color !== 'none' ? paint.color : ''
  const hex = paint.type === 'solid' && paint.color?.startsWith('#') ? paint.color.slice(1).toUpperCase() : ''
  const opacity = mixed ? '' : String(Math.round((paint.opacity ?? 1) * 100))
  const swatch = mixed ? 'transparent' : paintSwatch(doc, paint, fallback)
  const label = mixed ? '' : paintRowLabel(paint) || token
  const commitHex = (raw: string) => {
    setHexDraft(null)
    const trimmed = raw.trim()
    const namedToken = tokens?.find((item) => item.name === trimmed)
    if (namedToken) {
      onChange({ ...solidPaint(namedToken.name), opacity: paint.opacity })
      return
    }
    const body = trimmed.startsWith('#') ? trimmed.slice(1) : trimmed
    if (/^[0-9a-fA-F]{3}$|^[0-9a-fA-F]{6}$/.test(body)) {
      const color = body.length === 3 ? `#${body.split('').map((item) => item + item).join('')}`.toLowerCase() : `#${body.toLowerCase()}`
      onChange({ ...solidPaint(color), opacity: paint.opacity })
    }
  }
  return (
    <div className="flex items-center gap-1">
      <button
        type="button"
        title={paint.visible === false ? 'Show' : 'Hide'}
        aria-label={paint.visible === false ? 'Show paint' : 'Hide paint'}
        onClick={() => onChange({ ...paint, visible: paint.visible === false ? undefined : false })}
        className="flex h-7 w-7 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
      >
        {paint.visible === false ? <EyeOff size={14} /> : <Eye size={14} />}
      </button>
      <div className="grid h-7 min-w-0 flex-1 grid-cols-[1fr_auto] items-center overflow-hidden rounded border border-koma-border bg-koma-bg focus-within:border-koma-fg/40">
        <div className="flex h-7 min-w-0 items-center">
          <button
            ref={swatchRef}
            type="button"
            title="Color"
            aria-label="Color"
            aria-expanded={open}
            onClick={() => setOpen((current) => !current)}
            className="ml-2 h-4 w-4 flex-none rounded-full border border-koma-border"
            style={{ background: swatch }}
          />
          {named || token ? (
            <span className="min-w-0 flex-1 truncate px-1.5 text-[12px] text-koma-fg">{mixed ? '—' : label}</span>
          ) : (
            <input
              aria-label="Hex"
              value={mixed ? '' : hexDraft ?? hex}
              placeholder={mixed ? '—' : '000000'}
              onChange={(event) => {
                const raw = event.target.value
                const body = raw.startsWith('#') ? raw.slice(1) : raw
                if (/^[0-9a-fA-F]{6}$/.test(body)) {
                  setHexDraft(null)
                  onChange({ ...solidPaint(`#${body.toLowerCase()}`), opacity: paint.opacity })
                  return
                }
                setHexDraft(raw)
              }}
              onBlur={() => {
                if (hexDraft != null) commitHex(hexDraft)
              }}
              className="h-7 min-w-0 flex-1 bg-transparent px-1.5 text-[12px] uppercase text-koma-fg outline-none"
            />
          )}
        </div>
        <div className="flex h-7 w-[52px] items-center border-l border-koma-border pl-1.5">
          <span className="text-[11px] text-koma-dim">%</span>
          <input
            aria-label="Opacity"
            defaultValue={opacity}
            key={opacity}
            placeholder={mixed ? '—' : undefined}
            onBlur={(event) => {
              const value = Number(event.target.value)
              if (!Number.isFinite(value)) return
              const next = Math.max(0, Math.min(100, value)) / 100
              onChange({ ...paint, opacity: next >= 1 ? undefined : next })
            }}
            className="h-7 min-w-0 flex-1 bg-transparent px-1 text-[12px] text-koma-fg outline-none"
          />
        </div>
      </div>
      {tokens?.length && onBind ? <TokenBindControl tokens={tokens} bound={bound} onBind={onBind} /> : null}
      {onRemove ? (
        <button type="button" title="Remove" aria-label="Remove color" className="flex h-7 w-7 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg" onClick={onRemove}>
          <Minus size={14} />
        </button>
      ) : null}
      {open ? (
        <DesignPaintPopup
          doc={doc}
          paint={paint}
          fallback={fallback}
          tokens={tokens}
          allowImage={allowImage}
          allowGradient={allowGradient}
          anchor={swatchRef.current}
          onChange={onChange}
          onClose={() => setOpen(false)}
          onStoreImage={onStoreImage}
        />
      ) : null}
    </div>
  )
}

export function PaintRow({ label, value, mixed, fallback, resolved, tokens, weight, onWeight, onChange, doc }: { label: string; value: string; mixed?: boolean; fallback: string; resolved?: string; tokens?: DesignToken[]; weight?: { value: number; mixed?: boolean }; onWeight?: (value: number) => void; onChange: (next: string | null) => void; doc?: DesignDoc }) {
  const paint = solidPaint(value && value !== 'none' ? value : fallback)
  return (
    <div className="flex flex-col gap-1">
      <ColorRow
        doc={doc ?? { version: 2, modes: ['light'], mode: 'light', grid: 8, tokens: [], screens: [], components: [] }}
        paint={value === 'none' ? solidPaint('none') : paint}
        fallback={resolved?.startsWith('#') ? resolved : fallback}
        tokens={tokens}
        mixed={mixed}
        allowImage={false}
        allowGradient={false}
        onChange={(next) => onChange(next.color && next.color !== 'none' ? next.color : 'none')}
      />
      {weight && onWeight ? (
        <div className="w-16">
          <GeomField label="W" ariaLabel="Weight" value={weight.value} mixed={weight.mixed} onChange={onWeight} />
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
                  ? `flex h-7 items-center justify-center rounded-md ${grow ? 'min-w-0 flex-1' : 'w-7 flex-none'} ${pressed ? 'bg-koma-hover text-koma-fg' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`
                  : `h-7 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-1.5 text-[12px] ${pressed ? 'border-koma-fg/40 text-koma-fg' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`
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
  onStoreImage,
}: {
  label: string
  doc: DesignDoc
  node: DesignNode
  field: 'fill' | 'stroke'
  mixed?: boolean
  tokens?: DesignToken[]
  onChange: (node: DesignNode) => void
  onPickImage?: () => void
  onStoreImage?: (hash: string, bytes: Uint8Array, mime: string) => void
}) {
  const fallback = field === 'fill' ? SHAPE_FILL : '#1c1c1c'
  const stored = nodePaints(node, field)
  const paints = stored.length ? stored : [solidPaint(fallback)]
  const implicit = !stored.length
  if (mixed) {
    return <PaintRow label={label} doc={doc} value="" mixed fallback={fallback} tokens={tokens} onChange={(next) => onChange(setNodeSolid(node, field, next))} />
  }
  return (
    <div className="flex flex-col gap-2">
      {paints.map((paint, index) => (
        <div key={`${paint.type}-${index}`} className="flex items-start gap-1">
          {paints.length > 1 ? (
            <div className="flex flex-col">
              <button type="button" title="Move up" aria-label="Move up" disabled={index === 0} className="h-4 w-5 text-[10px] text-koma-dim disabled:opacity-30" onClick={() => onChange(reorderNodePaint(node, field, index, index - 1))}>↑</button>
              <button type="button" title="Move down" aria-label="Move down" disabled={index === paints.length - 1} className="h-4 w-5 text-[10px] text-koma-dim disabled:opacity-30" onClick={() => onChange(reorderNodePaint(node, field, index, index + 1))}>↓</button>
            </div>
          ) : null}
          <div className="min-w-0 flex-1">
            <ColorRow
              doc={doc}
              paint={paint}
              fallback={fallback}
              tokens={tokens}
              allowImage={field === 'fill'}
              bound={node.bindings?.[field]}
              onBind={tokens?.length ? (token) => onChange(bindNodeField(setNodePaints(node, field, implicit ? [paint] : paints), field, token)) : undefined}
              onChange={(next) => onChange(setNodePaints(node, field, implicit ? [next] : paints.map((item, at) => (at === index ? next : item))))}
              onRemove={() => onChange(implicit || paints.length <= 1 ? setNodeSolid(node, field, 'none') : setNodePaints(node, field, paints.filter((_, at) => at !== index)))}
              onStoreImage={onStoreImage}
            />
            {field === 'stroke' ? (
              <div className="mt-1 grid grid-cols-2 gap-1">
                <GeomField label="Width" ariaLabel="Stroke width" value={paint.width ?? node.strokeWidth ?? 1} onChange={(width) => onChange(setNodePaints(node, 'stroke', paints.map((item, at) => (at === index ? { ...item, width } : item))))} />
                <GeomField label="Dash" ariaLabel="Dash" value={paint.dash ?? 0} onChange={(dash) => onChange(setNodePaints(node, 'stroke', paints.map((item, at) => (at === index ? { ...item, dash: dash > 0 ? dash : undefined } : item))))} />
                <GeomField label="Gap" ariaLabel="Gap" value={paint.gap ?? 0} onChange={(gap) => onChange(setNodePaints(node, 'stroke', paints.map((item, at) => (at === index ? { ...item, gap: gap > 0 ? gap : undefined } : item))))} />
                <KomaSelect
                  aria-label="Align"
                  value={paint.align ?? node.strokeAlign ?? 'center'}
                  onChange={(event) => {
                    const align = event.target.value
                    if (align !== 'inside' && align !== 'center' && align !== 'outside') return
                    onChange(setNodePaints(node, 'stroke', paints.map((item, at) => (at === index ? { ...item, align: align === 'center' ? undefined : align } : item))))
                  }}
                  className="h-7 w-full px-1.5 text-[12px]"
                >
                  <option value="inside">Inside</option>
                  <option value="center">Center</option>
                  <option value="outside">Outside</option>
                </KomaSelect>
                <GeomField label="Top" ariaLabel="Stroke top" value={paint.top ?? node.strokeTop ?? paint.width ?? node.strokeWidth ?? 1} onChange={(top) => onChange(setNodePaints(node, 'stroke', paints.map((item, at) => (at === index ? { ...item, top } : item))))} />
                <GeomField label="Right" ariaLabel="Stroke right" value={paint.right ?? node.strokeRight ?? paint.width ?? node.strokeWidth ?? 1} onChange={(right) => onChange(setNodePaints(node, 'stroke', paints.map((item, at) => (at === index ? { ...item, right } : item))))} />
                <GeomField label="Bottom" ariaLabel="Stroke bottom" value={paint.bottom ?? node.strokeBottom ?? paint.width ?? node.strokeWidth ?? 1} onChange={(bottom) => onChange(setNodePaints(node, 'stroke', paints.map((item, at) => (at === index ? { ...item, bottom } : item))))} />
                <GeomField label="Left" ariaLabel="Stroke left" value={paint.left ?? node.strokeLeft ?? paint.width ?? node.strokeWidth ?? 1} onChange={(left) => onChange(setNodePaints(node, 'stroke', paints.map((item, at) => (at === index ? { ...item, left } : item))))} />
                <KomaSelect aria-label="Cap" value={paint.capStart ?? node.strokeStart ?? node.strokeCap ?? 'none'} onChange={(event) => onChange(setNodePaints(node, 'stroke', paints.map((item, at) => (at === index ? { ...item, capStart: event.target.value === 'none' ? undefined : event.target.value as DesignPaint['capStart'] } : item))))} className="h-7 w-full px-1.5 text-[12px]">
                  <option value="none">Cap none</option>
                  <option value="round">Cap round</option>
                  <option value="square">Cap square</option>
                </KomaSelect>
                <KomaSelect aria-label="End cap" value={paint.capEnd ?? node.strokeEnd ?? node.strokeCap ?? 'none'} onChange={(event) => onChange(setNodePaints(node, 'stroke', paints.map((item, at) => (at === index ? { ...item, capEnd: event.target.value === 'none' ? undefined : event.target.value as DesignPaint['capEnd'] } : item))))} className="h-7 w-full px-1.5 text-[12px]">
                  <option value="none">End none</option>
                  <option value="round">End round</option>
                  <option value="square">End square</option>
                </KomaSelect>
                <KomaSelect aria-label="Start marker" value={paint.markerStart ?? node.strokeMarkerStart ?? 'none'} onChange={(event) => onChange(setNodePaints(node, 'stroke', paints.map((item, at) => (at === index ? { ...item, markerStart: event.target.value === 'none' ? undefined : event.target.value as DesignPaint['markerStart'] } : item))))} className="h-7 w-full px-1.5 text-[12px]">
                  <option value="none">Start none</option>
                  <option value="arrow">Start arrow</option>
                  <option value="dot">Start dot</option>
                </KomaSelect>
                <KomaSelect aria-label="End marker" value={paint.markerEnd ?? node.strokeMarkerEnd ?? 'none'} onChange={(event) => onChange(setNodePaints(node, 'stroke', paints.map((item, at) => (at === index ? { ...item, markerEnd: event.target.value === 'none' ? undefined : event.target.value as DesignPaint['markerEnd'] } : item))))} className="h-7 w-full px-1.5 text-[12px]">
                  <option value="none">End none</option>
                  <option value="arrow">End arrow</option>
                  <option value="dot">End dot</option>
                </KomaSelect>
                <KomaSelect aria-label="Join" value={paint.join ?? node.strokeJoin ?? 'miter'} onChange={(event) => onChange(setNodePaints(node, 'stroke', paints.map((item, at) => (at === index ? { ...item, join: event.target.value === 'miter' ? undefined : event.target.value as DesignPaint['join'] } : item))))} className="col-span-2 h-7 w-full px-1.5 text-[12px]">
                  <option value="miter">Join miter</option>
                  <option value="round">Join round</option>
                  <option value="bevel">Join bevel</option>
                </KomaSelect>
              </div>
            ) : null}
          </div>
        </div>
      ))}
    </div>
  )
}

