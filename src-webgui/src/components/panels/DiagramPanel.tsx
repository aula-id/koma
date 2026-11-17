import { useEffect, useState, type MouseEvent as ReactMouseEvent } from 'react'
import { File, FolderOpen, Pencil, Trash2, Workflow } from 'lucide-react'
import { AccordionSection } from '../AccordionSection'
import { BrailleSpinner } from '../BrailleSpinner'
import { AddBtn, Empty, EmptyState, IconBtn, InlineNameInput, NoProjectState, NoWorkspaceState, WelcomeLink } from './helpers'
import { Select } from './form'
import { useKoma } from '../../store/koma'
import { fileKey, isMissingCodingDirError, type FileTreeEntry } from '../../store/coding'
import { SHAPE_MIME, diagramFileName, isDiagramPath } from '../../lib/diagram'
import { addDiagramFileToChat, copyDiagramFileMermaid } from '../../lib/diagramChat'
import { DiagramRefMenuItems } from '../DiagramVisual'
import { DIAGRAM_PALETTE, ShapePreview, type DiagramPaletteEntry } from '../diagram/DiagramShapePreview'
import { isEditorTabActive } from '../../lib/welcome'
import { pathBasename as rootLabel } from '../../lib/pathDisplay'

const PALETTE_GROUPS = ['Basic', 'Flowchart', 'Containers', 'Database'] as const

const EMPTY_ROOTS: string[] = []



function ShapeTile({ entry }: { entry: DiagramPaletteEntry }) {
  return (
    <div
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData(SHAPE_MIME, entry.id)
        e.dataTransfer.setData('text/plain', entry.id)
        try {
          e.dataTransfer.effectAllowed = 'copy'
        } catch {
          /* ignore */
        }
      }}
      className="flex h-9 w-14 cursor-grab items-center justify-center rounded p-0.5 text-koma-dim hover:bg-koma-hover hover:text-koma-fg active:cursor-grabbing"
      title={`Drag ${entry.label} onto the canvas`}
    >
      <ShapePreview entry={entry} />
    </div>
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
  const diagramActive = useKoma((s) => isEditorTabActive('diagram', s.ui))
  const activeTabId = useKoma((s) => s.ui.activeTabId)

  const [browseFiles, setBrowseFiles] = useState(false)
  const [shapesOpen, setShapesOpen] = useState(true)
  const [diagramsOpen, setDiagramsOpen] = useState(true)
  const [creating, setCreating] = useState(false)
  const [renaming, setRenaming] = useState<string | null>(null)
  const [deleting, setDeleting] = useState<string | null>(null)
  const [menu, setMenu] = useState<null | { x: number; y: number; path: string }>(null)

  useEffect(() => setBrowseFiles(false), [activeTabId])

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

  if (sessionId === null) return <NoProjectState feature="Diagram" />
  if (workdir.length === 0) return <NoWorkspaceState />

  const editorMode = diagramActive && !browseFiles
  const startCreate = () => { setCreating(true); setRenaming(null); setDeleting(null) }

  const files: FileTreeEntry[] = (dir?.entries ?? []).filter((e) => !e.isDir && isDiagramPath(e.path))
  const gone = isMissingCodingDirError(dir?.error)
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
            onChange={(root) => setActiveCodingRoot(root, { closeTabs: true })}
            disabled={creating || renaming != null || deleting != null}
          />
        </div>
        {diagramActive && (
          <button
            type="button"
            aria-label="Files"
            aria-pressed={browseFiles}
            title={browseFiles ? 'Back to editor tools' : 'Show diagram files'}
            onClick={() => setBrowseFiles((v) => !v)}
            className={`flex h-6 w-6 flex-none items-center justify-center rounded hover:bg-koma-hover hover:text-koma-fg ${browseFiles ? 'text-koma-fg' : 'text-koma-dim'}`}
          >
            <FolderOpen size={14} />
          </button>
        )}
      </div>
      {editorMode && (
      <AccordionSection title="Shapes" open={shapesOpen} onToggle={() => setShapesOpen((v) => !v)} fill={false} shrink>
        <div className="flex flex-col gap-1 px-2 py-1.5">
          {PALETTE_GROUPS.map((group) => (
            <div key={group} className="flex flex-col gap-0.5">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-koma-dim">{group}</span>
              <div className="flex flex-wrap items-center gap-1">
                {DIAGRAM_PALETTE.filter((entry) => entry.group === group).map((entry) => (
                  <ShapeTile key={entry.id} entry={entry} />
                ))}
              </div>
            </div>
          ))}
        </div>
      </AccordionSection>
      )}
      {!editorMode && (
      <AccordionSection
        title="Diagrams"
        open={diagramsOpen}
        minHeight={120}
        onToggle={() => setDiagramsOpen((v) => !v)}
        action={<AddBtn label="New diagram" text="New" onClick={startCreate} />}
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
                        <div className="flex max-w-0 flex-none items-center overflow-hidden opacity-0 transition-[max-width,opacity] duration-100 group-hover:max-w-[56px] group-hover:opacity-100 group-has-[:focus-visible]:max-w-[56px] group-has-[:focus-visible]:opacity-100">
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
              <EmptyState
                icon={Workflow}
                title="No diagrams yet"
                primary={{ label: 'New diagram', onClick: startCreate }}
                footer={<WelcomeLink which="diagram" />}
              >
                Diagrams are drawings of how things connect, like a flowchart. Ask Koma to draw one, or make your own.
              </EmptyState>
            ) : null}
          </>
        )}
      </AccordionSection>
      )}
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
