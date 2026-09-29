import { useEffect, useState, type FormEvent, type KeyboardEvent, type MouseEvent as ReactMouseEvent } from 'react'
import { Check, File, Pencil, Trash2, X } from 'lucide-react'
import { AccordionSection } from '../AccordionSection'
import { BrailleSpinner } from '../BrailleSpinner'
import { AddBtn, Empty, IconBtn } from './helpers'
import { Select } from './form'
import { useKoma } from '../../store/koma'
import { fileKey, type FileTreeEntry } from '../../store/coding'
import { SHAPE_MIME, diagramFileName, isDiagramPath, type DiagramKind } from '../../lib/diagram'
import { addDiagramFileToChat, copyDiagramFileMermaid } from '../../lib/diagramChat'
import { DiagramRefMenuItems } from '../DiagramVisual'

const EMPTY_ROOTS: string[] = []

const SHAPES: { kind: DiagramKind; label: string }[] = [
  { kind: 'rect', label: 'Rectangle' },
  { kind: 'text', label: 'Text' },
  { kind: 'ellipse', label: 'Ellipse' },
  { kind: 'diamond', label: 'Diamond' },
]

function rootLabel(root: string): string {
  const parts = root.split('/').filter(Boolean)
  return parts[parts.length - 1] || root
}

function missingDir(error: string | null | undefined): boolean {
  if (!error) return false
  return /no such file|not found|os error 2/i.test(error)
}

function InlineNameInput({
  initial,
  placeholder,
  onSubmit,
  onCancel,
}: {
  initial: string
  placeholder: string
  onSubmit: (value: string) => void
  onCancel: () => void
}) {
  const [value, setValue] = useState(initial)
  useEffect(() => {
    const el = document.getElementById('diagram-name-input') as HTMLInputElement | null
    el?.focus()
    el?.select()
  }, [])
  const commit = () => {
    const v = value.trim()
    if (!v) {
      onCancel()
      return
    }
    onSubmit(v)
  }
  return (
    <form
      className="flex min-w-0 flex-1 items-center gap-1"
      onSubmit={(e: FormEvent) => {
        e.preventDefault()
        commit()
      }}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
    >
      <input
        id="diagram-name-input"
        value={value}
        placeholder={placeholder}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e: KeyboardEvent<HTMLInputElement>) => {
          if (e.key === 'Escape') {
            e.preventDefault()
            onCancel()
          }
        }}
        onBlur={() => {
          if (!value.trim()) onCancel()
        }}
        className="h-5 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-1.5 text-[12px] text-koma-fg outline-none focus:border-koma-fg/40"
      />
      <IconBtn label="Confirm" tone="emerald" onClick={commit}>
        <Check size={12} />
      </IconBtn>
      <IconBtn label="Cancel" tone="red" onClick={onCancel}>
        <X size={12} />
      </IconBtn>
    </form>
  )
}

function ShapeTile({ kind, label }: { kind: DiagramKind; label: string }) {
  return (
    <div
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData(SHAPE_MIME, kind)
        e.dataTransfer.setData('text/plain', kind)
        try {
          e.dataTransfer.effectAllowed = 'copy'
        } catch {
          /* ignore */
        }
      }}
      className="flex cursor-grab items-center justify-center rounded p-0.5 text-koma-dim hover:bg-koma-hover hover:text-koma-fg active:cursor-grabbing"
      title={`Drag ${label} onto the canvas`}
    >
      <ShapePreview kind={kind} />
    </div>
  )
}

function ShapePreview({ kind }: { kind: DiagramKind }) {
  if (kind === 'ellipse') {
    return (
      <svg viewBox="0 0 64 36" className="h-8 w-16" aria-hidden="true">
        <ellipse cx="32" cy="18" rx="30" ry="15" fill="var(--color-koma-bg)" stroke="currentColor" strokeWidth="1.25" />
      </svg>
    )
  }
  if (kind === 'diamond') {
    return (
      <svg viewBox="0 0 40 36" className="h-8 w-10" aria-hidden="true">
        <polygon points="20,1.5 38.5,18 20,34.5 1.5,18" fill="var(--color-koma-bg)" stroke="currentColor" strokeWidth="1.25" />
      </svg>
    )
  }
  if (kind === 'text') {
    return <span className="flex h-8 items-center px-1 text-[13px] leading-none text-koma-fg/80">Text</span>
  }
  return (
    <span className="flex h-8 items-center justify-center rounded border border-current bg-koma-bg px-2 text-[11px] leading-none text-koma-fg/80">
      Heading
    </span>
  )
}

export function DiagramPanel() {
  const sessionId = useKoma((s) => s.session.id)
  const workdir = useKoma((s) => s.settingsValues?.workdir ?? EMPTY_ROOTS)
  const activeRoot = useKoma((s) => s.coding.activeRoot)
  const dir = useKoma((s) => (activeRoot ? s.coding.dirs[fileKey(activeRoot, '.koma')] : undefined))
  const docs = useKoma((s) => s.diagram.docs)
  const setActiveCodingRoot = useKoma((s) => s.setActiveCodingRoot)
  const refreshCodingDir = useKoma((s) => s.refreshCodingDir)
  const openDiagramTab = useKoma((s) => s.openDiagramTab)
  const createDiagramFile = useKoma((s) => s.createDiagramFile)
  const renameCodingItem = useKoma((s) => s.renameCodingItem)
  const deleteCodingItem = useKoma((s) => s.deleteCodingItem)
  const req = useKoma((s) => s.req)

  const [shapesOpen, setShapesOpen] = useState(true)
  const [diagramsOpen, setDiagramsOpen] = useState(true)
  const [creating, setCreating] = useState(false)
  const [renaming, setRenaming] = useState<string | null>(null)
  const [deleting, setDeleting] = useState<string | null>(null)
  const [menu, setMenu] = useState<null | { x: number; y: number; path: string }>(null)

  useEffect(() => {
    req({ r: 'GetSettings' })
  }, [req])

  useEffect(() => {
    if (workdir.length === 0) {
      if (activeRoot != null) setActiveCodingRoot(null)
      return
    }
    if (!activeRoot || !workdir.includes(activeRoot)) setActiveCodingRoot(workdir[0])
  }, [workdir, activeRoot, setActiveCodingRoot])

  useEffect(() => {
    if (!activeRoot) return
    refreshCodingDir(activeRoot, '.koma')
    setCreating(false)
    setRenaming(null)
    setDeleting(null)
    setMenu(null)
  }, [activeRoot, refreshCodingDir])

  useEffect(() => {
    if (!menu) return
    const close = () => setMenu(null)
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === 'Escape') close()
    }
    window.addEventListener('click', close)
    window.addEventListener('keydown', onKey)
    window.addEventListener('blur', close)
    return () => {
      window.removeEventListener('click', close)
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('blur', close)
    }
  }, [menu])

  if (sessionId === null) return <Empty>Open a project to use Diagram</Empty>
  if (workdir.length === 0) return <Empty>No workspaces configured. Add paths under Settings → Session → workdir.</Empty>

  const files: FileTreeEntry[] = (dir?.entries ?? []).filter((e) => !e.isDir && isDiagramPath(e.path))
  const gone = missingDir(dir?.error)
  const busy = (path: string) => {
    if (!activeRoot) return false
    return !!docs[fileKey(activeRoot, path)]?.saving
  }

  const submitCreate = (raw: string) => {
    if (!activeRoot) return
    const name = diagramFileName(raw)
    if (!name) return
    createDiagramFile(activeRoot, `.koma/${name}`)
    setCreating(false)
  }

  const submitRename = (path: string, raw: string) => {
    if (!activeRoot || busy(path)) return
    const name = diagramFileName(raw)
    if (!name) return
    const next = `.koma/${name}`
    if (next !== path) renameCodingItem(activeRoot, path, next)
    setRenaming(null)
  }

  const confirmDelete = (path: string) => {
    if (!activeRoot || busy(path)) return
    deleteCodingItem(activeRoot, path)
    setDeleting(null)
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-none items-center gap-1 px-2 py-1.5">
        <div className="min-w-0 flex-1" title={activeRoot ?? ''}>
          <Select
            value={activeRoot ?? workdir[0] ?? ''}
            options={workdir.map((r) => ({ value: r, label: rootLabel(r) }))}
            onChange={(root) => setActiveCodingRoot(root)}
            disabled={creating || renaming != null || deleting != null}
          />
        </div>
      </div>
      <AccordionSection title="Shapes" open={shapesOpen} onToggle={() => setShapesOpen((v) => !v)} fill={false}>
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5 px-2 py-1.5">
          {SHAPES.map((shape) => (
            <ShapeTile key={shape.kind} {...shape} />
          ))}
        </div>
      </AccordionSection>
      <AccordionSection
        title="Diagrams"
        open={diagramsOpen}
        onToggle={() => setDiagramsOpen((v) => !v)}
        action={<AddBtn label="New diagram" onClick={() => { setCreating(true); setRenaming(null); setDeleting(null) }} />}
      >
        {!activeRoot ? (
          <Empty>Select a workspace root</Empty>
        ) : dir?.loading && !dir.entries.length && !creating ? (
          <div className="flex items-center gap-2 px-3 py-2 text-[12px] text-koma-dim">
            <BrailleSpinner size={13} />
            <span>Loading…</span>
          </div>
        ) : dir?.error && !gone ? (
          <div className="px-3 py-2 text-[12px] text-koma-error">{dir.error}</div>
        ) : (
          <>
            {creating ? (
              <div className="flex h-7 min-w-0 items-center gap-1 px-2 text-[12px] text-koma-fg">
                <File size={13} className="flex-none opacity-70" />
                <InlineNameInput
                  initial=""
                  placeholder="name.diag"
                  onSubmit={submitCreate}
                  onCancel={() => setCreating(false)}
                />
              </div>
            ) : null}
            {files.length ? (
              files.map((entry) => {
                const dirty = activeRoot ? !!docs[fileKey(activeRoot, entry.path)]?.dirty : false
                if (deleting === entry.path) {
                  return (
                    <div
                      key={entry.path}
                      className="flex min-h-[28px] w-full items-center gap-2 bg-koma-error/15 px-2 text-[12px] font-medium text-koma-error"
                    >
                      <span className="min-w-0 flex-1 truncate">delete file?</span>
                      <button
                        type="button"
                        className="rounded px-1.5 py-0.5 text-koma-success hover:bg-koma-success/15"
                        onClick={() => confirmDelete(entry.path)}
                      >
                        yes
                      </button>
                      <button
                        type="button"
                        className="rounded px-1.5 py-0.5 text-koma-error hover:bg-koma-error/15"
                        onClick={() => setDeleting(null)}
                      >
                        no
                      </button>
                    </div>
                  )
                }
                return (
                  <div
                    key={entry.path}
                    className="group flex h-7 min-w-0 items-center gap-1 px-2 text-[12px] text-koma-fg hover:bg-koma-hover"
                    onContextMenu={(e: ReactMouseEvent) => {
                      e.preventDefault()
                      setMenu({ x: e.clientX, y: e.clientY, path: entry.path })
                    }}
                  >
                    {renaming === entry.path ? (
                      <>
                        <File size={13} className="flex-none opacity-70" />
                        <InlineNameInput
                          initial={entry.name}
                          placeholder="new name"
                          onSubmit={(name) => submitRename(entry.path, name)}
                          onCancel={() => setRenaming(null)}
                        />
                      </>
                    ) : (
                      <>
                        <button
                          type="button"
                          className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden text-left"
                          title={entry.path}
                          onClick={() => activeRoot && openDiagramTab(activeRoot, entry.path)}
                        >
                          <File size={13} className="flex-none opacity-70" />
                          <span className="min-w-0 flex-1 truncate">{entry.name}</span>
                        </button>
                        <div className="flex max-w-0 flex-none items-center overflow-hidden opacity-0 transition-[max-width,opacity] duration-100 group-hover:max-w-[56px] group-hover:opacity-100">
                          <IconBtn label="Rename" onClick={() => { setRenaming(entry.path); setCreating(false); setDeleting(null) }}>
                            <Pencil size={12} />
                          </IconBtn>
                          <IconBtn label="Delete" tone="red" onClick={() => { setDeleting(entry.path); setCreating(false); setRenaming(null) }}>
                            <Trash2 size={12} />
                          </IconBtn>
                        </div>
                        {dirty ? (
                          <span className="flex-none font-mono text-[11px] font-semibold text-koma-accent">M</span>
                        ) : null}
                      </>
                    )}
                  </div>
                )
              })
            ) : !creating ? (
              <Empty>No diagrams</Empty>
            ) : null}
          </>
        )}
      </AccordionSection>
      {menu ? (
        <div
          className="fixed z-[80] min-w-[140px] rounded border border-koma-border bg-koma-panel py-1 shadow-lg"
          style={{ left: menu.x, top: menu.y }}
          onClick={(e) => e.stopPropagation()}
          onContextMenu={(e) => e.preventDefault()}
        >
          {activeRoot ? (
            <>
              <DiagramRefMenuItems
                onAdd={() => {
                  const path = menu.path
                  setMenu(null)
                  if (activeRoot) void addDiagramFileToChat(activeRoot, path)
                }}
                onCopy={() => {
                  const path = menu.path
                  setMenu(null)
                  if (activeRoot) void copyDiagramFileMermaid(activeRoot, path)
                }}
              />
              <div className="my-1 border-t border-koma-border" />
            </>
          ) : null}
          <button
            type="button"
            className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[12px] text-koma-fg opacity-90 hover:bg-koma-hover"
            onClick={() => {
              setRenaming(menu.path)
              setCreating(false)
              setDeleting(null)
              setMenu(null)
            }}
          >
            <Pencil size={12} className="opacity-70" />
            Rename
          </button>
          <button
            type="button"
            className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[12px] text-koma-error opacity-90 hover:bg-koma-error/15"
            onClick={() => {
              setDeleting(menu.path)
              setCreating(false)
              setRenaming(null)
              setMenu(null)
            }}
          >
            <Trash2 size={12} className="opacity-70" />
            Delete
          </button>
        </div>
      ) : null}
    </div>
  )
}
