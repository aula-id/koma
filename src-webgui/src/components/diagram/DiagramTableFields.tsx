import { useRef, useState, type ReactNode } from 'react'
import { ChevronDown, ChevronRight, MoreHorizontal, Plus } from 'lucide-react'
import { MenuItem, Popover } from '../panels/Popover'
import {
  DOCUMENT_TYPES,
  TABLE_ROW_LIMIT,
  addChildRow,
  allRows,
  indentRow,
  isNestType,
  moveRowWithinSiblings,
  nextKey,
  outdentRow,
  removeRow,
  toggleCollapsed,
  updateRow,
  type DiagramKey,
  type DiagramRow,
} from '../../lib/diagram'

type TypeGroup = { label: string; types: string[] }

/** Types offered for document fields: scalars first, then nests, then arrays of scalars. */
const DOCUMENT_GROUPS: TypeGroup[] = [
  { label: 'Value', types: DOCUMENT_TYPES.filter((type) => type !== 'object' && type !== 'array') },
  { label: 'Nested', types: ['object', 'array<object>'] },
  { label: 'Array of', types: ['array<string>', 'array<number>', 'array<int>', 'array<objectId>', 'array<date>', 'array<mixed>'] },
]

/** Common SQL column types; any other type can be typed in. */
const SQL_GROUPS: TypeGroup[] = [
  { label: 'Number', types: ['int', 'bigint', 'serial', 'numeric', 'real'] },
  { label: 'Text', types: ['varchar(255)', 'text', 'char(36)', 'uuid'] },
  { label: 'Other', types: ['boolean', 'date', 'timestamp', 'timestamptz', 'json', 'jsonb', 'bytea'] },
]

function newRowId(): string {
  return `r${Date.now().toString(36)}${Math.random().toString(36).slice(2, 5)}`
}

function keyText(key: DiagramKey | undefined): string {
  return key === 'pk' ? 'PK' : key === 'fk' ? 'FK' : key === 'pk,fk' ? 'PK FK' : ''
}

/** The type of a field as a pill; the menu lists document types, or takes any typed name. */
function TypePill({ type, groups, onChange }: { type: string; groups: TypeGroup[]; onChange: (type: string | undefined) => void }) {
  const [open, setOpen] = useState(false)
  const [draft, setDraft] = useState('')
  const ref = useRef<HTMLButtonElement>(null)
  const pick = (next: string) => {
    onChange(next.trim() || undefined)
    setOpen(false)
  }
  return (
    <>
      <button
        ref={ref}
        type="button"
        title="Field type"
        onClick={() => {
          setDraft('')
          setOpen((current) => !current)
        }}
        className={`h-6 max-w-[45%] flex-none truncate rounded px-1.5 font-mono text-[11px] hover:bg-koma-hover ${
          type ? 'text-koma-accent' : 'text-koma-dim'
        }`}
      >
        {type || 'type'}
      </button>
      <Popover anchor={ref} open={open} onClose={() => setOpen(false)} width={180}>
        <div className="px-2 pb-1">
          <input
            autoFocus
            value={draft}
            placeholder="Custom type…"
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && draft.trim()) pick(draft)
            }}
            className="h-6 w-full rounded border border-koma-border bg-koma-bg px-1.5 font-mono text-[11px] text-koma-fg outline-none focus:border-koma-fg/40"
          />
        </div>
        {groups.map((group) => (
          <div key={group.label}>
            <div className="px-2.5 pt-1 text-[10px] uppercase tracking-wider text-koma-dim">{group.label}</div>
            {group.types.map((option) => (
              <MenuItem key={option} onPick={() => pick(option)}>
                <span className={`font-mono text-[11px] ${option === type ? 'text-koma-accent' : ''}`}>{option}</span>
              </MenuItem>
            ))}
          </div>
        ))}
      </Popover>
    </>
  )
}

/**
 * A table's columns or a document's fields: one line each with the key, the name and a type
 * pill. Document fields also nest (folding like on the canvas) and can be optional. Rarer
 * actions sit in a ⋯ menu that shows on hover.
 */
export function TableFields({
  rows,
  document,
  onRows,
  onTextDone,
}: {
  rows: DiagramRow[]
  /** Document tables nest and mark optional fields; SQL tables are flat columns. */
  document: boolean
  onRows: (rows: DiagramRow[], live: boolean) => void
  onTextDone: () => void
}) {
  const total = allRows(rows).length
  const set = (id: string, patch: Partial<DiagramRow>, live = false) => onRows(updateRow(rows, id, (row) => ({ ...row, ...patch })), live)
  const render = (list: DiagramRow[], depth: number): ReactNode =>
    list.map((row, index) => (
      <div key={row.id}>
        <FieldLine
          row={row}
          depth={depth}
          document={document}
          first={index === 0}
          last={index === list.length - 1}
          full={total >= TABLE_ROW_LIMIT}
          onName={(name) => set(row.id, { name }, true)}
          onType={(type) => set(row.id, { type })}
          onOptional={() => set(row.id, { optional: row.optional ? undefined : true })}
          onKey={() => set(row.id, { key: nextKey(row.key) })}
          onFold={() => onRows(toggleCollapsed(rows, row.id), false)}
          onAddInside={() => onRows(addChildRow(rows, row.id, { id: newRowId(), name: '' }), false)}
          onIndent={() => onRows(indentRow(rows, row.id), false)}
          onOutdent={() => onRows(outdentRow(rows, row.id), false)}
          onMove={(by) => onRows(moveRowWithinSiblings(rows, row.id, index + by), false)}
          onRemove={() => onRows(removeRow(rows, row.id), false)}
          onTextDone={onTextDone}
        />
        {row.children?.length && !row.collapsed ? (
          // The nest's own fields sit under a guide rule: a table in the table.
          <div className="relative">
            <span className="pointer-events-none absolute bottom-1 top-0 w-px bg-koma-border" style={{ left: depth * 14 + 9 }} />
            {render(row.children, depth + 1)}
          </div>
        ) : null}
      </div>
    ))
  return (
    <div className="flex flex-col">
      <div className="flex flex-col rounded border border-koma-border bg-koma-bg/40 py-0.5">{render(rows, 0)}</div>
      <button
        type="button"
        className="mt-1 flex h-7 items-center justify-center gap-1 rounded text-[12px] text-koma-dim hover:bg-koma-hover hover:text-koma-fg disabled:opacity-40"
        disabled={total >= TABLE_ROW_LIMIT}
        onClick={() => onRows([...rows, { id: newRowId(), name: '' }], false)}
      >
        <Plus size={12} />
        {document ? 'Add field' : 'Add column'}
      </button>
    </div>
  )
}

function FieldLine({
  row,
  depth,
  document,
  first,
  last,
  full,
  onName,
  onType,
  onOptional,
  onKey,
  onFold,
  onAddInside,
  onIndent,
  onOutdent,
  onMove,
  onRemove,
  onTextDone,
}: {
  row: DiagramRow
  depth: number
  document: boolean
  first: boolean
  last: boolean
  full: boolean
  onName: (name: string) => void
  onType: (type: string | undefined) => void
  onOptional: () => void
  onKey: () => void
  onFold: () => void
  onAddInside: () => void
  onIndent: () => void
  onOutdent: () => void
  onMove: (by: number) => void
  onRemove: () => void
  onTextDone: () => void
}) {
  const [menu, setMenu] = useState(false)
  const more = useRef<HTMLButtonElement>(null)
  const nest = document && isNestType(row.type)
  const key = keyText(row.key)
  const run = (action: () => void) => () => {
    action()
    setMenu(false)
  }
  return (
    <div className="group flex h-7 items-center gap-0.5 pr-1" style={{ paddingLeft: depth * 14 + 2 }}>
      {document ? (
        <button
          type="button"
          aria-label={row.collapsed ? 'Unfold' : 'Fold'}
          disabled={!row.children?.length}
          onClick={onFold}
          className="flex h-5 w-4 flex-none items-center justify-center rounded text-koma-dim hover:text-koma-fg disabled:invisible"
        >
          {row.collapsed ? <ChevronRight size={12} /> : <ChevronDown size={12} />}
        </button>
      ) : null}
      {key || !document ? (
        <button
          type="button"
          title={key ? 'Key. Click to change.' : 'No key. Click for PK, again for FK.'}
          onClick={onKey}
          className={`h-4 flex-none rounded px-1 text-[9px] font-semibold leading-4 ${
            key ? 'bg-koma-accent/15 text-koma-accent' : 'w-6 text-koma-dim opacity-40 hover:opacity-100'
          }`}
        >
          {key || '—'}
        </button>
      ) : null}
      <input
        value={row.name}
        placeholder={document ? 'field' : 'column'}
        aria-label={document ? 'Field name' : 'Column name'}
        onFocus={onTextDone}
        onBlur={onTextDone}
        onChange={(e) => onName(e.target.value)}
        className="h-6 min-w-0 flex-1 rounded bg-transparent px-1 text-[12px] text-koma-fg outline-none hover:bg-koma-hover focus:bg-koma-bg focus:ring-1 focus:ring-koma-border"
      />
      {document ? (
        <button
          type="button"
          aria-pressed={!!row.optional}
          title={row.optional ? 'Optional (some documents lack it). Click to make required.' : 'Required. Click to make optional.'}
          onClick={onOptional}
          className={`h-5 w-4 flex-none rounded text-[12px] font-semibold ${
            row.optional ? 'text-koma-accent' : 'text-koma-dim opacity-0 hover:opacity-100 group-hover:opacity-60'
          }`}
        >
          ?
        </button>
      ) : null}
      <TypePill type={row.type ?? ''} groups={document ? DOCUMENT_GROUPS : SQL_GROUPS} onChange={onType} />
      <button
        ref={more}
        type="button"
        aria-label="More field actions"
        onClick={() => setMenu((open) => !open)}
        className={`flex h-5 w-5 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg ${menu ? 'opacity-100' : 'opacity-0 group-hover:opacity-100 focus:opacity-100'}`}
      >
        <MoreHorizontal size={13} />
      </button>
      <Popover anchor={more} open={menu} onClose={() => setMenu(false)} width={140}>
        {nest ? (
          <MenuItem disabled={full} onPick={run(onAddInside)}>
            Add inside
          </MenuItem>
        ) : null}
        <MenuItem disabled={first} onPick={run(() => onMove(-1))}>
          Move up
        </MenuItem>
        <MenuItem disabled={last} onPick={run(() => onMove(1))}>
          Move down
        </MenuItem>
        {/* Right nests a field into the one above it; left takes it out of its parent. */}
        {document ? (
          <MenuItem disabled={first} onPick={run(onIndent)}>
            Move right <span className="ml-auto text-[10px] text-koma-dim">⌥→</span>
          </MenuItem>
        ) : null}
        {document ? (
          <MenuItem disabled={depth === 0} onPick={run(onOutdent)}>
            Move left <span className="ml-auto text-[10px] text-koma-dim">⌥←</span>
          </MenuItem>
        ) : null}
        <MenuItem onPick={run(onKey)}>
          Key <span className="ml-auto text-[10px] text-koma-dim">{key || 'none'}</span>
        </MenuItem>
        <div className="my-1 h-px bg-koma-border" />
        <MenuItem danger onPick={run(onRemove)}>
          Remove
        </MenuItem>
      </Popover>
    </div>
  )
}
