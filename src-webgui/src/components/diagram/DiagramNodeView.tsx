import { memo } from 'react'
import { inlineToHtml } from '../../lib/markdownNote'
import { GROUP_TITLE, TABLE_HEADER, TABLE_INDENT, TABLE_ROW, nodeStyle, tableLines, type DiagramKey, type DiagramNode } from '../../lib/diagram'
import { DiagramShapeSvg } from './DiagramShapeSvg'

export type DiagramCellField = 'name' | 'type'
export type DiagramCell = { rowId: string; field: DiagramCellField }
/** What a key press in a table cell asks for. Indent and outdent nest fields in document tables. */
export type DiagramCellMove = 'next' | 'prev' | 'add' | 'delete' | 'done' | 'indent' | 'outdent'

export type DiagramNodeEdit = {
  text: (id: string, text: string) => void
  done: () => void
  cellText: (id: string, cell: DiagramCell, text: string) => void
  cellMove: (id: string, cell: DiagramCell, move: DiagramCellMove) => void
}

/** Width of a table's key column (PK, FK). */
export const TABLE_KEY_WIDTH = 36

export function keyLabel(key: DiagramKey | undefined): string {
  return key === 'pk' ? 'PK' : key === 'fk' ? 'FK' : key === 'pk,fk' ? 'PK FK' : ''
}

/**
 * One shape in doc coordinates. The world layer scales it, so it never reads the zoom.
 * Pointer handling lives on the canvas (geometric hits); only the editors take input.
 */
export const DiagramNodeView = memo(function DiagramNodeView({
  node,
  highlight,
  editing,
  cell,
  edit,
}: {
  node: DiagramNode
  /** Accent outline: the start of a click-to-connect, or a connection target. */
  highlight: boolean
  /** Editing starts with the whole label selected, or with the caret after typed text. */
  editing: false | 'select' | 'end'
  /** The table cell being edited, when `editing` is set on a table. */
  cell: DiagramCell | null
  edit: DiagramNodeEdit
}) {
  const paint = nodeStyle(node)
  const fill = paint.fill ? paint.fillColor || 'var(--color-koma-panel)' : 'none'
  const stroke = highlight ? 'var(--color-koma-accent)' : paint.stroke ? paint.strokeColor || 'var(--color-koma-border)' : 'none'
  const color = paint.textColor || 'var(--color-koma-fg)'
  if (node.kind === 'table') {
    return <TableView node={node} fill={fill} stroke={stroke} color={color} editing={editing} cell={cell} edit={edit} />
  }
  if (node.kind === 'group') {
    const line = stroke === 'none' ? 'var(--color-koma-border)' : stroke
    return (
      <div
        className="absolute"
        style={{
          left: node.x,
          top: node.y,
          width: node.w,
          height: node.h,
          transform: node.rotation ? `rotate(${node.rotation}deg)` : undefined,
        }}
      >
        <svg className="absolute inset-0 h-full w-full overflow-visible">
          <rect width={node.w} height={node.h} fill={fill} stroke={line} />
          <path d={`M0 ${GROUP_TITLE} H${node.w}`} fill="none" stroke={line} />
        </svg>
        {editing ? (
          <div className="absolute inset-x-0 top-0 flex items-center" style={{ height: GROUP_TITLE }}>
            <LabelEditor text={node.text} mode={editing} color={color} onText={(text) => edit.text(node.id, text)} onDone={edit.done} />
          </div>
        ) : (
          <span
            className="absolute inset-x-0 top-0 z-10 truncate px-2 text-center text-[12px] font-semibold"
            style={{ color, height: GROUP_TITLE, lineHeight: `${GROUP_TITLE}px` }}
          >
            {node.text || 'Group'}
          </span>
        )}
      </div>
    )
  }
  return (
    <div
      className="absolute flex items-center justify-center"
      style={{
        left: node.x,
        top: node.y,
        width: node.w,
        height: node.h,
        transform: node.rotation ? `rotate(${node.rotation}deg)` : undefined,
      }}
    >
      <svg className="absolute inset-0 h-full w-full overflow-visible">
        <DiagramShapeSvg kind={node.kind} w={node.w} h={node.h} fill={fill} stroke={stroke} />
      </svg>
      {editing ? (
        <LabelEditor text={node.text} mode={editing} color={color} onText={(text) => edit.text(node.id, text)} onDone={edit.done} />
      ) : (
        <span
          className="z-10 max-h-full overflow-hidden whitespace-pre-wrap break-words px-2 text-center text-[12px] leading-4"
          style={{ color }}
          dangerouslySetInnerHTML={{ __html: inlineToHtml(node.text || ' ') }}
        />
      )}
    </div>
  )
})

function LabelEditor({
  text,
  mode,
  color,
  onText,
  onDone,
}: {
  text: string
  mode: 'select' | 'end'
  color: string
  onText: (text: string) => void
  onDone: () => void
}) {
  return (
    <textarea
      autoFocus
      defaultValue={text}
      rows={Math.max(1, text.split('\n').length)}
      className="pointer-events-auto z-10 max-h-full w-full resize-none overflow-hidden border-0 bg-transparent px-2 text-center text-[12px] leading-4 shadow-none outline-none"
      style={{ color }}
      onFocus={(e) => {
        const field = e.currentTarget
        if (mode === 'select') field.select()
        else field.setSelectionRange(field.value.length, field.value.length)
      }}
      onPointerDown={(e) => e.stopPropagation()}
      onChange={(e) => {
        e.currentTarget.rows = Math.max(1, e.currentTarget.value.split('\n').length)
        onText(e.target.value)
      }}
      onKeyDown={(e) => {
        // Shift+Enter breaks the line; Enter and Escape finish, as in draw.io.
        if ((e.key === 'Enter' && !e.shiftKey) || e.key === 'Escape') {
          e.preventDefault()
          onDone()
        }
      }}
      onBlur={onDone}
    />
  )
}

/** draw.io-style entity: a bold header, then one row per column with its key, name and type. */
function TableView({
  node,
  fill,
  stroke,
  color,
  editing,
  cell,
  edit,
}: {
  node: DiagramNode
  fill: string
  stroke: string
  color: string
  editing: false | 'select' | 'end'
  cell: DiagramCell | null
  edit: DiagramNodeEdit
}) {
  const lines = tableLines(node)
  const document = node.dialect === 'document'
  const line = stroke === 'none' ? 'transparent' : stroke
  return (
    <div
      className="absolute overflow-hidden rounded text-[12px]"
      style={{
        left: node.x,
        top: node.y,
        width: node.w,
        height: node.h,
        background: fill === 'none' ? 'transparent' : fill,
        boxShadow: `inset 0 0 0 1px ${line}`,
        color,
      }}
    >
      <div className="relative flex items-center justify-center px-2 font-semibold" style={{ height: TABLE_HEADER, borderBottom: `1px solid ${line}` }}>
        {editing && !cell ? (
          <LabelEditor text={node.text} mode={editing} color={color} onText={(text) => edit.text(node.id, text)} onDone={edit.done} />
        ) : (
          <span className="truncate">{node.text || ' '}</span>
        )}
        {document ? (
          <span className="absolute right-2 text-[10px] font-normal opacity-50" title="Document collection">
            {'{ }'}
          </span>
        ) : null}
      </div>
      {lines.length ? (
        lines.map(({ row, depth }) => {
          const editingField = editing && cell?.rowId === row.id ? cell.field : null
          const nest = !!row.children?.length
          return (
            <div
              key={row.id}
              className="relative flex items-center"
              style={{
                height: TABLE_ROW,
                // Nested fields sit on a faintly deeper tint, one step per level: a table in the table.
                background: depth ? `color-mix(in srgb, currentColor ${Math.min(depth, 4) * 4}%, transparent)` : undefined,
              }}
            >
              {depth ? (
                <span
                  className="pointer-events-none absolute bottom-0 top-0 w-px opacity-40"
                  style={{ left: TABLE_KEY_WIDTH + (depth - 1) * TABLE_INDENT + TABLE_INDENT / 2, background: line === 'transparent' ? 'currentColor' : line }}
                />
              ) : null}
              <span className="flex-none text-center text-[10px] font-semibold opacity-70" style={{ width: TABLE_KEY_WIDTH }}>
                {keyLabel(row.key)}
              </span>
              {document ? (
                <span className="flex flex-none items-center justify-center opacity-60" style={{ marginLeft: depth * TABLE_INDENT, width: TABLE_INDENT }}>
                  {nest ? (row.collapsed ? '▸' : '▾') : ''}
                </span>
              ) : null}
              {editingField === 'name' ? (
                <CellEditor value={row.name} cell={{ rowId: row.id, field: 'name' }} node={node} edit={edit} className="min-w-0 flex-1" />
              ) : (
                <span className={`min-w-0 flex-1 truncate ${row.key?.includes('pk') ? 'underline' : ''}`}>
                  {row.name || ' '}
                  {row.optional ? <span className="opacity-50">?</span> : null}
                </span>
              )}
              {editingField === 'type' ? (
                <CellEditor value={row.type ?? ''} cell={{ rowId: row.id, field: 'type' }} node={node} edit={edit} className="w-2/5 text-right" />
              ) : (
                <span className="max-w-[40%] flex-none truncate pr-2 text-right opacity-60">{row.type ?? ''}</span>
              )}
            </div>
          )
        })
      ) : (
        <div className="flex items-center justify-center opacity-50" style={{ height: TABLE_ROW }}>
          No columns
        </div>
      )}
    </div>
  )
}

/**
 * One table cell being typed into. Tab walks the cells, Enter adds a row below,
 * Backspace on an empty name removes the row, Escape finishes.
 */
function CellEditor({
  value,
  cell,
  node,
  edit,
  className,
}: {
  value: string
  cell: DiagramCell
  node: DiagramNode
  edit: DiagramNodeEdit
  className: string
}) {
  return (
    <input
      autoFocus
      defaultValue={value}
      data-diagram-cell=""
      className={`pointer-events-auto border-0 bg-transparent px-0 pr-2 text-[12px] shadow-none outline-none ${className}`}
      onFocus={(e) => e.currentTarget.select()}
      onPointerDown={(e) => e.stopPropagation()}
      onChange={(e) => edit.cellText(node.id, cell, e.target.value)}
      onKeyDown={(e) => {
        const move: DiagramCellMove | null =
          e.altKey && e.key === 'ArrowRight'
            ? 'indent'
            : e.altKey && e.key === 'ArrowLeft'
              ? 'outdent'
              : e.key === 'Tab'
            ? e.shiftKey
              ? 'prev'
              : 'next'
            : e.key === 'Enter'
              ? 'add'
              : e.key === 'Escape'
                ? 'done'
                : e.key === 'Backspace' && cell.field === 'name' && !e.currentTarget.value
                  ? 'delete'
                  : null
        if (!move) return
        e.preventDefault()
        edit.cellMove(node.id, cell, move)
      }}
      onBlur={(e) => {
        // Tab to another cell of the table refocuses right away; only leaving the table ends editing.
        if (!(e.relatedTarget as HTMLElement | null)?.closest('[data-diagram-cell]')) edit.done()
      }}
    />
  )
}
