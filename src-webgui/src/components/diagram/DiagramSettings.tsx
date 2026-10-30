import { useState, type ReactNode } from 'react'
import { tableToJsonSample, tableToJsonSchema } from '../../lib/diagramJson'
import { TableFields } from './DiagramTableFields'
import { Toggle } from '../panels/form'
import { InlineMarkdownInput, MarkdownNote } from '../MarkdownNote'
import { diagramShapeFolder, shapeHeadingFor } from '../../lib/diagramNotes'
import {
  edgeStyle,
  nodeStyle,
  type DiagramArrow,
  type DiagramDoc,
  type DiagramEdge,
  type DiagramJump,
  type DiagramNode,
  type DiagramRoute,
  type DiagramDialect,
  type DiagramRow,
} from '../../lib/diagram'

export type DiagramPanelSelection = { type: 'node' | 'edge'; id: string }

function LineGlyph({ children }: { children: ReactNode }) {
  return (
    <svg viewBox="0 0 28 14" className="h-3.5 w-7 flex-none" aria-hidden="true">
      {children}
    </svg>
  )
}

const LINE_HEADS: { value: DiagramArrow; name: string }[] = [
  { value: 'none', name: 'No' },
  { value: 'arrow', name: 'Arrow' },
  { value: 'open', name: 'Open arrow' },
  { value: 'diamond', name: 'Diamond' },
  { value: 'diamondOpen', name: 'Open diamond' },
  { value: 'dot', name: 'Dot' },
  { value: 'circle', name: 'Circle' },
  { value: 'bar', name: 'Bar' },
  { value: 'erOne', name: 'One' },
  { value: 'erMandOne', name: 'Exactly one' },
  { value: 'erMany', name: 'Many' },
  { value: 'erOneToMany', name: 'One or many' },
  { value: 'erZeroToOne', name: 'Zero or one' },
  { value: 'erZeroToMany', name: 'Zero or many' },
]

function ArrowGlyph({ side, kind }: { side: 'start' | 'end'; kind: DiagramArrow }) {
  const p = (x: number) => (side === 'end' ? x : 28 - x)
  const er = kind.startsWith('er')
  const reach = er ? 25 : kind === 'none' ? 24 : kind === 'arrow' || kind === 'open' ? 16 : kind === 'diamond' || kind === 'diamondOpen' ? 14 : kind === 'bar' ? 23 : 18
  const shaft = `M${p(4)} 7 H${p(reach)}`
  const triangle = `M${p(15.5)} 3 L${p(24)} 7 L${p(15.5)} 11`
  const diamond = `M${p(24)} 7 L${p(19)} 3 L${p(14)} 7 L${p(19)} 11 Z`
  const crow = `M${p(25)} 3 L${p(17)} 7 L${p(25)} 11`
  const bar = (x: number) => <path d={`M${p(x)} 3 V11`} fill="none" stroke="currentColor" strokeWidth="1.5" />
  const ring = (x: number) => <circle cx={p(x)} cy="7" r="2.5" fill="var(--color-koma-panel)" stroke="currentColor" strokeWidth="1.5" />
  return (
    <LineGlyph>
      <path d={shaft} fill="none" stroke="currentColor" strokeWidth="1.5" />
      {kind === 'arrow' ? <path d={`${triangle} Z`} fill="currentColor" /> : null}
      {kind === 'open' ? <path d={triangle} fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="miter" /> : null}
      {kind === 'diamond' ? <path d={diamond} fill="currentColor" /> : null}
      {kind === 'diamondOpen' ? <path d={diamond} fill="none" stroke="currentColor" strokeWidth="1.5" /> : null}
      {kind === 'dot' ? <circle cx={p(21)} cy="7" r="3" fill="currentColor" /> : null}
      {kind === 'circle' ? <circle cx={p(21)} cy="7" r="3" fill="none" stroke="currentColor" strokeWidth="1.5" /> : null}
      {kind === 'bar' ? bar(23) : null}
      {kind === 'erMany' || kind === 'erOneToMany' || kind === 'erZeroToMany' ? (
        <path d={crow} fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="miter" />
      ) : null}
      {kind === 'erOne' ? bar(21) : null}
      {kind === 'erMandOne' ? (
        <>
          {bar(19)}
          {bar(22)}
        </>
      ) : null}
      {kind === 'erOneToMany' ? bar(14) : null}
      {kind === 'erZeroToOne' ? (
        <>
          {bar(22)}
          {ring(15)}
        </>
      ) : null}
      {kind === 'erZeroToMany' ? ring(12) : null}
    </LineGlyph>
  )
}

function Choices<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string
  value: T
  options: { value: T; label: string; icon?: ReactNode }[]
  onChange: (value: T) => void
}) {
  // Icon rows wrap into a grid past four; short text rows (widths) stay on one line.
  const grid = options.length > (options.some((option) => option.icon) ? 4 : 6)
  return (
    <div className="flex flex-col gap-1">
      <span className="text-[10px] font-semibold uppercase tracking-wider text-koma-dim">{label}</span>
      <div className={grid ? 'grid grid-cols-4 gap-0.5 rounded border border-koma-border p-0.5' : 'flex rounded border border-koma-border p-0.5'}>
        {options.map((option) => {
          const on = value === option.value
          return (
            <button
              key={option.value}
              type="button"
              title={option.label}
              aria-label={option.label}
              aria-pressed={on}
              onClick={() => onChange(option.value)}
              className={`flex h-7 items-center justify-center rounded ${grid ? '' : 'min-w-0 flex-1 px-1'} ${
                on ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
              }`}
            >
              {option.icon ?? <span className="truncate text-[11px]">{option.label}</span>}
            </button>
          )
        })}
      </div>
    </div>
  )
}

function ColorWell({
  label,
  value,
  fallback,
  picker,
  onChange,
}: {
  label: string
  value: string
  fallback: string
  picker: string
  onChange: (hex: string) => void
}) {
  return (
    <label className="relative h-7 w-7 flex-none cursor-pointer overflow-hidden rounded border border-koma-border" title={label}>
      <span className="absolute inset-0" style={{ background: value || fallback }} />
      <input
        type="color"
        aria-label={label}
        value={/^#[0-9a-fA-F]{6}$/.test(value) ? value : picker}
        className="absolute inset-0 cursor-pointer opacity-0"
        onChange={(e) => onChange(e.target.value)}
      />
    </label>
  )
}

function PaintRow({
  label,
  on,
  color,
  fallback,
  picker,
  onToggle,
  onColor,
}: {
  label: string
  on: boolean
  color: string
  fallback: string
  picker: string
  onToggle: (on: boolean) => void
  onColor: (hex: string) => void
}) {
  return (
    <div className="flex h-8 items-center gap-2">
      <span className="min-w-0 flex-1 text-[12px] text-koma-fg">{label}</span>
      <Toggle on={on} onChange={onToggle} />
      <ColorWell label={`${label} color`} value={color} fallback={fallback} picker={picker} onChange={onColor} />
    </div>
  )
}

export function Properties({
  doc,
  root,
  diagPath,
  legacyAssetDir,
  selection,
  onEdge,
  onNode,
  onText,
  onEdgeText,
  onDetail,
  onTextDone,
  onRows,
  count = 0,
}: {
  doc: DiagramDoc
  root: string
  diagPath: string
  legacyAssetDir: string
  selection: DiagramPanelSelection | null
  onEdge: (id: string, patch: Partial<DiagramEdge>) => void
  onNode: (id: string, patch: Partial<DiagramNode>) => void
  onText: (id: string, text: string) => void
  onEdgeText: (id: string, text: string) => void
  onDetail: (kind: 'node' | 'edge', id: string, detail: string) => void
  onTextDone: () => void
  /** Table columns. `live` groups typing into one undo step, like labels. */
  onRows?: (id: string, rows: DiagramRow[], live: boolean) => void
  /** Items selected when more than one is picked. The panel edits one at a time. */
  count?: number
}) {
  const [tab, setTab] = useState<'style' | 'detail'>('style')
  const node = selection?.type === 'node' ? doc.nodes.find((item) => item.id === selection.id) : undefined
  const edge = selection?.type === 'edge' ? doc.edges.find((item) => item.id === selection.id) : undefined
  const target = node ?? edge
  const selected = node ? 'node' : edge ? 'edge' : null
  const shapeKind = node ? 'node' : edge ? 'edge' : null
  const assetDir =
    shapeKind && target ? diagramShapeFolder(diagPath, shapeKind, target.id) ?? '' : ''
  const detailTitle =
    target && shapeKind
      ? node
        ? `Detail · ${shapeHeadingFor(node.text, node.id)} (\`${node.id}\`)`
        : `Detail · line (\`${edge!.id}\`)`
      : 'Detail'
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-8 flex-none items-center gap-1 border-b border-koma-border px-2">
        {selected ? (
          <>
            <PanelTab label={node ? 'Card' : 'Line'} on={tab === 'style'} onClick={() => setTab('style')} />
            <PanelTab label="Detail" on={tab === 'detail'} onClick={() => setTab('detail')} />
          </>
        ) : (
          <span className="px-1 text-[12px] text-koma-fg">Properties</span>
        )}
      </div>
      {selected && tab === 'detail' && target && shapeKind ? (
        <>
          <p className="flex-none border-b border-koma-border px-3 py-1.5 text-[10px] text-koma-dim">{detailTitle}</p>
          <MarkdownNote
            key={target.id}
            value={target.detail ?? ''}
            root={root}
            assetDir={assetDir}
            legacyAssetDir={legacyAssetDir}
            onChange={(detail) => onDetail(shapeKind, target.id, detail)}
            onDone={onTextDone}
          />
        </>
      ) : node ? (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <CardSettings
            node={node}
            onChange={(patch) => onNode(node.id, patch)}
            onText={(text) => onText(node.id, text)}
            onRows={(rows, live) => onRows?.(node.id, rows, live)}
            onTextDone={onTextDone}
          />
        </div>
      ) : edge ? (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <LineSettings edge={edge} onChange={(patch) => onEdge(edge.id, patch)} onText={(text) => onEdgeText(edge.id, text)} onTextDone={onTextDone} />
        </div>
      ) : (
        <p className="px-3 py-2 text-[12px] text-koma-dim">{count > 1 ? `${count} items selected` : 'Select a card or a line'}</p>
      )}
    </div>
  )
}

function PanelTab({ label, on, onClick }: { label: string; on: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      aria-pressed={on}
      onClick={onClick}
      className={`h-6 rounded px-2 text-[12px] ${on ? 'bg-koma-bg text-koma-fg' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`}
    >
      {label}
    </button>
  )
}

function CardSettings({
  node,
  onChange,
  onText,
  onRows,
  onTextDone,
}: {
  node: DiagramNode
  onChange: (patch: Partial<DiagramNode>) => void
  onText: (text: string) => void
  onRows: (rows: DiagramRow[], live: boolean) => void
  onTextDone: () => void
}) {
  const style = nodeStyle(node)
  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="flex flex-col gap-1">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-koma-dim">Label</span>
        <div className="flex items-start gap-2">
          <InlineMarkdownInput
            key={node.id}
            text={node.text}
            aria-label="Label"
            className="min-h-7 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-2 py-1 text-[12px] text-koma-fg outline-none focus:border-koma-fg/40 [&_strong]:font-semibold [&_em]:italic [&_code]:rounded [&_code]:bg-koma-panel2 [&_code]:px-0.5"
            style={{ color: style.textColor || undefined }}
            onFocus={onTextDone}
            onBlur={onTextDone}
            onChange={onText}
          />
          <ColorWell
            label="Label color"
            value={style.textColor}
            fallback="var(--color-koma-fg)"
            picker="#c8d3f5"
            onChange={(textColor) => onChange({ textColor })}
          />
        </div>
      </div>
      <PaintRow
        label="Fill"
        on={style.fill}
        color={style.fillColor}
        fallback="var(--color-koma-panel)"
        picker="#1a1d27"
        onToggle={(fill) => onChange({ fill })}
        onColor={(fillColor) => onChange({ fillColor, fill: true })}
      />
      <PaintRow
        label="Border"
        on={style.stroke}
        color={style.strokeColor}
        fallback="var(--color-koma-border)"
        picker="#8b93b8"
        onToggle={(stroke) => onChange({ stroke })}
        onColor={(strokeColor) => onChange({ strokeColor, stroke: true })}
      />
      {node.kind === 'table' && node.dialect === 'document' ? <JsonCopyRow node={node} /> : null}
      {node.kind === 'table' ? (
        <ColumnSettings
          rows={node.rows ?? []}
          dialect={node.dialect ?? 'sql'}
          onRows={onRows}
          onDialect={(dialect, rows) => onChange({ dialect: dialect === 'document' ? 'document' : undefined, rows })}
          onTextDone={onTextDone}
        />
      ) : null}
    </div>
  )
}

/** Copy a document table out as a sample document or as a MongoDB validator. */
function JsonCopyRow({ node }: { node: DiagramNode }) {
  const [copied, setCopied] = useState<'json' | 'schema' | null>(null)
  const copy = (kind: 'json' | 'schema') => {
    const text = kind === 'json' ? tableToJsonSample(node) : tableToJsonSchema(node)
    void navigator.clipboard.writeText(text).then(
      () => {
        setCopied(kind)
        window.setTimeout(() => setCopied((current) => (current === kind ? null : current)), 1200)
      },
      () => setCopied(null),
    )
  }
  const button = 'h-7 flex-1 rounded border border-koma-border bg-koma-bg text-[11px] text-koma-fg hover:border-koma-fg/40'
  return (
    <div className="flex gap-1">
      <button type="button" className={button} title="A sample document shaped like this collection" onClick={() => copy('json')}>
        {copied === 'json' ? 'Copied' : 'Copy JSON'}
      </button>
      <button type="button" className={button} title="A MongoDB $jsonSchema validator for this collection" onClick={() => copy('schema')}>
        {copied === 'schema' ? 'Copied' : 'Copy JSON Schema'}
      </button>
    </div>
  )
}

/** SQL tables are flat: nested document fields become dotted columns (`address.city`). */
function flattenRows(rows: DiagramRow[], prefix = ''): DiagramRow[] {
  return rows.flatMap((row) => {
    const name = prefix ? `${prefix}.${row.name}` : row.name
    const own: DiagramRow = { id: row.id, name, ...(row.type ? { type: row.type } : {}), ...(row.key ? { key: row.key } : {}) }
    return [own, ...(row.children ? flattenRows(row.children, name) : [])]
  })
}

/**
 * A table's columns, or a document's fields: key, name and type per row, with add, move
 * and remove. Document tables nest fields to any depth: indent a field into the one above,
 * add a field inside an object or array, mark fields optional.
 */
function ColumnSettings({
  rows,
  dialect,
  onRows,
  onDialect,
  onTextDone,
}: {
  rows: DiagramRow[]
  dialect: DiagramDialect
  onRows: (rows: DiagramRow[], live: boolean) => void
  onDialect: (dialect: DiagramDialect, rows: DiagramRow[]) => void
  onTextDone: () => void
}) {
  const document = dialect === 'document'
  return (
    <div className="flex flex-col gap-1">
      <Choices
        label="Table"
        value={dialect}
        options={[
          { value: 'sql' as DiagramDialect, label: 'SQL' },
          { value: 'document', label: 'Document' },
        ]}
        onChange={(next) => onDialect(next, next === 'sql' ? flattenRows(rows) : rows)}
      />
      <span className="text-[10px] font-semibold uppercase tracking-wider text-koma-dim">{document ? 'Fields' : 'Columns'}</span>
      <TableFields rows={rows} document={document} onRows={onRows} onTextDone={onTextDone} />
    </div>
  )
}

function LineSettings({
  edge,
  onChange,
  onText,
  onTextDone,
}: {
  edge: DiagramEdge
  onChange: (patch: Partial<DiagramEdge>) => void
  onText: (text: string) => void
  onTextDone: () => void
}) {
  const style = edgeStyle(edge)
  return (
    <div className="flex flex-col gap-3 p-3">
      <label className="flex flex-col gap-1">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-koma-dim">Label</span>
        <input
          key={edge.id}
          value={edge.text ?? ''}
          placeholder="Label"
          aria-label="Label"
          onFocus={onTextDone}
          onChange={(e) => onText(e.target.value)}
          onBlur={onTextDone}
          className="h-7 w-full rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none focus:border-koma-fg/40"
        />
      </label>
      <PaintRow
        label="Stroke"
        on={style.stroke}
        color={style.color}
        fallback="var(--color-koma-fg)"
        picker="#c8d3f5"
        onToggle={(stroke) => onChange({ stroke: stroke ? undefined : false })}
        onColor={(color) => onChange({ color, stroke: undefined })}
      />
      <Choices
        label="Start"
        value={style.start}
        options={LINE_HEADS.map((head) => ({
          value: head.value,
          label: `${head.name} start`,
          icon: <ArrowGlyph side="start" kind={head.value} />,
        }))}
        onChange={(start) => onChange({ start })}
      />
      <Choices
        label="End"
        value={style.end}
        options={LINE_HEADS.map((head) => ({
          value: head.value,
          label: `${head.name} end`,
          icon: <ArrowGlyph side="end" kind={head.value} />,
        }))}
        onChange={(end) => onChange({ end })}
      />
      <Choices
        label="Pattern"
        value={style.dash}
        options={[
          {
            value: 'solid',
            label: 'Solid',
            icon: (
              <LineGlyph>
                <path d="M3 7 H25" fill="none" stroke="currentColor" strokeWidth="1.6" />
              </LineGlyph>
            ),
          },
          {
            value: 'dashed',
            label: 'Dashed',
            icon: (
              <LineGlyph>
                <path d="M3 7 H25" fill="none" stroke="currentColor" strokeWidth="1.6" strokeDasharray="5 3" />
              </LineGlyph>
            ),
          },
          {
            value: 'dotted',
            label: 'Dotted',
            icon: (
              <LineGlyph>
                <path d="M4 7 H24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeDasharray="0.1 4" />
              </LineGlyph>
            ),
          },
        ]}
        onChange={(dash) => onChange({ dash })}
      />
      <Choices
        label="Width"
        value={String(style.width)}
        options={[
          { value: '1', label: '1' },
          { value: '1.5', label: '1.5' },
          { value: '2', label: '2' },
          { value: '3', label: '3' },
          { value: '4', label: '4' },
          { value: '6', label: '6' },
        ]}
        onChange={(width) => onChange({ width: Number(width) })}
      />
      <Choices
        label="Corner"
        value={style.corner}
        options={[
          { value: 'sharp', label: 'Sharp' },
          { value: 'rounded', label: 'Round' },
        ]}
        onChange={(corner) => onChange({ corner })}
      />
      <Choices
        label="Route"
        value={style.route}
        options={[
          { value: 'orthogonal' satisfies DiagramRoute, label: 'Elbow' },
          { value: 'straight', label: 'Straight' },
          { value: 'curved', label: 'Curved' },
          { value: 'entity', label: 'ER' },
        ]}
        onChange={(route) => onChange({ route, bends: [] })}
      />
      <Choices
        label="Jumps"
        value={style.jump}
        options={[
          { value: 'none' satisfies DiagramJump, label: 'None' },
          { value: 'arc', label: 'Arc' },
          { value: 'gap', label: 'Gap' },
        ]}
        onChange={(jump) => onChange({ jump })}
      />
    </div>
  )
}
