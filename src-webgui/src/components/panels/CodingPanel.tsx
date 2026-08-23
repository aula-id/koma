import { useEffect, useMemo, useRef, useState, type DragEvent, type FormEvent, type KeyboardEvent, type MouseEvent as ReactMouseEvent } from 'react'
import {
  Check,
  ChevronDown,
  ChevronRight,
  Download,
  File,
  Folder,
  FolderOpen,
  FilePlus,
  FolderPlus,
  Pencil,
  RefreshCw,
  Trash2,
  X,
} from 'lucide-react'
import { useKoma } from '../../store/koma'
import {
  baseName,
  fileKey,
  isPathOrDescendant,
  parentDirPath,
  remapPath,
  type FileTreeEntry,
} from '../../store/coding'
import { BrailleSpinner } from '../BrailleSpinner'
import { Empty, IconBtn } from './helpers'
import { Segmented, Select } from './form'
import { CodingSearchPanel } from './CodingSearchPanel'

function joinPath(dir: string, name: string): string {
  if (!dir) return name
  if (!name) return dir
  return `${dir.replace(/\/+$/, '')}/${name.replace(/^\/+/, '')}`
}

function rootLabel(root: string): string {
  const parts = root.split('/').filter(Boolean)
  return parts[parts.length - 1] || root
}

function cleanName(raw: string): string | null {
  const cleaned = raw.trim().replace(/^\/+|\/+$/g, '')
  if (!cleaned || cleaned.includes('..') || cleaned.includes('/')) return null
  return cleaned
}

type Draft =
  | { kind: 'create'; dirPath: string; item: 'file' | 'dir' }
  | { kind: 'rename'; path: string }
  | { kind: 'delete'; path: string; isDir: boolean }

type TreeNodeProps = {
  root: string
  entry: FileTreeEntry
  depth: number
  expanded: Set<string>
  draft: Draft | null
  dropTargetPath: string | null
  onToggle: (path: string) => void
  onOpenFile: (path: string) => void
  onRefresh: (path: string) => void
  onStartCreate: (dirPath: string, item: 'file' | 'dir') => void
  onStartRename: (path: string) => void
  onStartDelete: (path: string, isDir: boolean) => void
  onDownload: (path: string) => void
  onCancelDraft: () => void
  onSubmitCreate: (dirPath: string, item: 'file' | 'dir', name: string) => void
  onSubmitRename: (path: string, name: string) => void
  onConfirmDelete: (path: string) => void
  onExternalDragOver: (e: DragEvent, dirPath: string) => void
  onExternalDrop: (e: DragEvent, dirPath: string) => void
  onExternalDragLeave: (e: DragEvent, dirPath: string) => void
  onOpenContextMenu: (e: ReactMouseEvent, path: string, isDir: boolean) => void
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
  const ref = useRef<HTMLInputElement | null>(null)

  useEffect(() => {
    ref.current?.focus()
    ref.current?.select()
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
      onClick={(e) => e.stopPropagation()}
      onMouseDown={(e) => e.stopPropagation()}
    >
      <input
        ref={ref}
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
          // Prefer explicit cancel/submit buttons; blur just cancels empty drafts.
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

function CreateDraftRow({
  depth,
  item,
  onSubmit,
  onCancel,
}: {
  depth: number
  item: 'file' | 'dir'
  onSubmit: (name: string) => void
  onCancel: () => void
}) {
  const pad = 8 + depth * 12
  return (
    <div
      className="flex h-7 min-w-0 items-center gap-1 pr-1 text-[12px] text-koma-fg"
      style={{ paddingLeft: pad }}
    >
      <span className="w-5 flex-none" />
      {item === 'dir' ? (
        <Folder size={13} className="flex-none text-koma-accent opacity-80" />
      ) : (
        <File size={13} className="flex-none opacity-70" />
      )}
      <InlineNameInput
        initial=""
        placeholder={item === 'dir' ? 'folder name' : 'file name'}
        onSubmit={onSubmit}
        onCancel={onCancel}
      />
    </div>
  )
}

function TreeNode({
  root,
  entry,
  depth,
  expanded,
  draft,
  dropTargetPath,
  onToggle,
  onOpenFile,
  onRefresh,
  onStartCreate,
  onStartRename,
  onStartDelete,
  onDownload,
  onCancelDraft,
  onSubmitCreate,
  onSubmitRename,
  onConfirmDelete,
  onExternalDragOver,
  onExternalDrop,
  onExternalDragLeave,
  onOpenContextMenu,
}: TreeNodeProps) {
  const key = fileKey(root, entry.path)
  const dirState = useKoma((s) => (entry.isDir ? s.coding.dirs[key] : null))
  const dirty = useKoma((s) => (!entry.isDir ? !!s.coding.files[key]?.dirty : false))
  const isNew = useKoma((s) => {
    if (entry.isDir) return false
    const f = s.coding.files[key]
    return !!f?.dirty && f.savedContent === null
  })
  const isOpen = entry.isDir && expanded.has(entry.path)
  const pad = 8 + depth * 12
  const renaming = draft?.kind === 'rename' && draft.path === entry.path
  const deleting = draft?.kind === 'delete' && draft.path === entry.path
  const createHere =
    draft?.kind === 'create' && draft.dirPath === entry.path && entry.isDir ? draft : null
  // Drop target is the directory itself, or the parent dir when hovering a file.
  const dropDir = entry.isDir ? entry.path : parentDirPath(entry.path)
  const isDropTarget = dropTargetPath !== null && dropTargetPath === dropDir

  if (deleting) {
    return (
      <div
        className="flex min-h-[28px] w-full items-center gap-2 bg-koma-error/15 px-2 text-[12px] font-medium text-koma-error"
        onMouseDown={(e) => e.stopPropagation()}
        onClick={(e) => e.stopPropagation()}
      >
        <span className="min-w-0 flex-1 truncate">
          delete {entry.isDir ? 'folder' : 'file'}?
        </span>
        <span className="flex flex-none items-center gap-1">
          <button
            type="button"
            autoFocus
            className="rounded px-1.5 py-0.5 text-koma-success hover:bg-koma-success/15"
            onMouseDown={(e) => e.stopPropagation()}
            onClick={(e) => { e.stopPropagation(); onConfirmDelete(entry.path) }}
          >
            yes
          </button>
          <button
            type="button"
            className="rounded px-1.5 py-0.5 text-koma-error hover:bg-koma-error/15"
            onMouseDown={(e) => e.stopPropagation()}
            onClick={(e) => { e.stopPropagation(); onCancelDraft() }}
          >
            no
          </button>
        </span>
      </div>
    )
  }

  return (
    <div>
      <div
        className={`group flex h-7 min-w-0 items-center gap-1 pr-1 text-[12px] text-koma-fg hover:bg-koma-hover ${
          isDropTarget ? 'bg-koma-accent/15 ring-1 ring-inset ring-koma-accent/40' : ''
        }`}
        style={{ paddingLeft: pad }}
        data-coding-row=""
        onDragOver={(e) => onExternalDragOver(e, dropDir)}
        onDrop={(e) => onExternalDrop(e, dropDir)}
        onDragLeave={(e) => onExternalDragLeave(e, dropDir)}
        onContextMenu={(e) => onOpenContextMenu(e, entry.path, entry.isDir)}
      >
        {entry.isDir ? (
          <button
            type="button"
            onClick={() => onToggle(entry.path)}
            className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-dim hover:text-koma-fg"
            aria-label={isOpen ? 'Collapse' : 'Expand'}
          >
            {isOpen ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
          </button>
        ) : (
          <span className="w-5 flex-none" />
        )}

        {renaming ? (
          <>
            {entry.isDir ? (
              <Folder size={13} className="flex-none text-koma-accent opacity-80" />
            ) : (
              <File size={13} className="flex-none opacity-70" />
            )}
            <InlineNameInput
              initial={entry.name}
              placeholder="new name"
              onSubmit={(name) => onSubmitRename(entry.path, name)}
              onCancel={onCancelDraft}
            />
          </>
        ) : (
          <>
            <button
              type="button"
              onClick={() => (entry.isDir ? onToggle(entry.path) : onOpenFile(entry.path))}
              className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden text-left"
              title={entry.path || entry.name}
            >
              {entry.isDir ? (
                isOpen ? (
                  <FolderOpen size={13} className="flex-none text-koma-accent opacity-80" />
                ) : (
                  <Folder size={13} className="flex-none text-koma-accent opacity-80" />
                )
              ) : (
                <File size={13} className="flex-none opacity-70" />
              )}
              {/* Truncates at the panel wall while idle. Hover expands the action
                  gutter below, which shrinks this flex slot so ellipsis moves to
                  the button edge instead of a permanently reserved gutter. */}
              <span className="min-w-0 flex-1 truncate">{entry.name}</span>
            </button>

            {/* Idle: max-w-0 so actions take no flex width (no premature ellipsis).
                Hover: expand and show. opacity alone would still reserve space. */}
            {!draft ? (
              <div className="flex max-w-0 flex-none items-center overflow-hidden opacity-0 transition-[max-width,opacity] duration-100 group-hover:max-w-[168px] group-hover:opacity-100">
                {entry.isDir && (
                  <>
                    <IconBtn label="New file" onClick={() => onStartCreate(entry.path, 'file')}>
                      <FilePlus size={12} />
                    </IconBtn>
                    <IconBtn label="New folder" onClick={() => onStartCreate(entry.path, 'dir')}>
                      <FolderPlus size={12} />
                    </IconBtn>
                    <IconBtn label="Refresh" onClick={() => onRefresh(entry.path)}>
                      <RefreshCw size={12} />
                    </IconBtn>
                  </>
                )}
                {!entry.isDir && (
                  <IconBtn label="Download" onClick={() => onDownload(entry.path)}>
                    <Download size={12} />
                  </IconBtn>
                )}
                <IconBtn label="Rename" onClick={() => onStartRename(entry.path)}>
                  <Pencil size={12} />
                </IconBtn>
                <IconBtn
                  label="Delete"
                  tone="red"
                  onClick={() => onStartDelete(entry.path, entry.isDir)}
                >
                  <Trash2 size={12} />
                </IconBtn>
              </div>
            ) : null}

            {dirty ? (
              <span
                className={`flex-none font-mono text-[11px] font-semibold ${
                  isNew ? 'text-koma-success' : 'text-koma-accent'
                }`}
              >
                {isNew ? 'A' : 'M'}
              </span>
            ) : null}
          </>
        )}
      </div>

      {entry.isDir && isOpen && (
        <div>
          {createHere ? (
            <CreateDraftRow
              depth={depth + 1}
              item={createHere.item}
              onSubmit={(name) => onSubmitCreate(entry.path, createHere.item, name)}
              onCancel={onCancelDraft}
            />
          ) : null}
          {dirState?.loading && !dirState.entries.length ? (
            <div
              className="flex h-7 items-center gap-2 text-[11px] text-koma-dim"
              style={{ paddingLeft: pad + 20 }}
            >
              <BrailleSpinner size={12} />
              <span>Loading…</span>
            </div>
          ) : dirState?.error ? (
            <div className="px-2 py-1 text-[11px] text-koma-error" style={{ paddingLeft: pad + 20 }}>
              {dirState.error}
            </div>
          ) : dirState?.entries?.length ? (
            dirState.entries.map((child) => (
              <TreeNode
                key={fileKey(root, child.path)}
                root={root}
                entry={child}
                depth={depth + 1}
                expanded={expanded}
                draft={draft}
                dropTargetPath={dropTargetPath}
                onToggle={onToggle}
                onOpenFile={onOpenFile}
                onRefresh={onRefresh}
                onStartCreate={onStartCreate}
                onStartRename={onStartRename}
                onStartDelete={onStartDelete}
                onDownload={onDownload}
                onCancelDraft={onCancelDraft}
                onSubmitCreate={onSubmitCreate}
                onSubmitRename={onSubmitRename}
                onConfirmDelete={onConfirmDelete}
                onExternalDragOver={onExternalDragOver}
                onExternalDrop={onExternalDrop}
                onExternalDragLeave={onExternalDragLeave}
                onOpenContextMenu={onOpenContextMenu}
              />
            ))
          ) : dirState && !dirState.loading && !createHere ? (
            <div
              className="px-2 py-1 text-[11px] text-koma-dim opacity-60"
              style={{ paddingLeft: pad + 20 }}
            >
              Empty
            </div>
          ) : null}
        </div>
      )}
    </div>
  )
}

// Sidebar panel for the Coding view: workspace root picker + lazy file tree.
const EMPTY_ROOTS: string[] = []

export function CodingPanel() {
  const sessionId = useKoma((s) => s.session.id)
  const settingsValues = useKoma((s) => s.settingsValues)
  const workdir = settingsValues?.workdir ?? EMPTY_ROOTS
  const activeRoot = useKoma((s) => s.coding.activeRoot)
  const dirs = useKoma((s) => s.coding.dirs)
  const setActiveCodingRoot = useKoma((s) => s.setActiveCodingRoot)
  const openCodingFile = useKoma((s) => s.openCodingFile)
  const refreshCodingDir = useKoma((s) => s.refreshCodingDir)
  const createCodingItem = useKoma((s) => s.createCodingItem)
  const renameCodingItem = useKoma((s) => s.renameCodingItem)
  const deleteCodingItem = useKoma((s) => s.deleteCodingItem)
  const uploadCodingFile = useKoma((s) => s.uploadCodingFile)
  const downloadCodingFile = useKoma((s) => s.downloadCodingFile)
  const req = useKoma((s) => s.req)

  const [expanded, setExpanded] = useState<Set<string>>(() => new Set(['']))
  const [draft, setDraft] = useState<Draft | null>(null)
  const [dropTargetPath, setDropTargetPath] = useState<string | null>(null)
  const [sideTab, setSideTab] = useState<'files' | 'search'>('files')
  const [ctxMenu, setCtxMenu] = useState<null | {
    x: number
    y: number
    path: string
    isDir: boolean
  }>(null)

  useEffect(() => {
    req({ r: 'GetSettings' })
  }, [req])

  useEffect(() => {
    if (workdir.length === 0) {
      if (activeRoot != null) setActiveCodingRoot(null)
      return
    }
    if (!activeRoot || !workdir.includes(activeRoot)) {
      setActiveCodingRoot(workdir[0])
    }
  }, [workdir, activeRoot, setActiveCodingRoot])

  useEffect(() => {
    if (!draft) return
    const onKeyDown = (e: globalThis.KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        setDraft(null)
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [draft])

  useEffect(() => {
    if (!ctxMenu) return
    const close = () => setCtxMenu(null)
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
  }, [ctxMenu])

  useEffect(() => {
    if (!activeRoot) return
    refreshCodingDir(activeRoot, '')
    setExpanded(new Set(['']))
    setDraft(null)
    setDropTargetPath(null)
    setCtxMenu(null)
  }, [activeRoot, refreshCodingDir])

  const rootKey = activeRoot ? fileKey(activeRoot, '') : null
  const rootDir = rootKey ? dirs[rootKey] : null
  const roots = useMemo(() => workdir.slice(), [workdir])
  const rootCreate =
    draft?.kind === 'create' && draft.dirPath === '' ? draft : null

  const hasExternalFiles = (e: DragEvent) => {
    const dt = e.dataTransfer
    if (!dt) return false
    if (dt.files && dt.files.length > 0) return true
    if (dt.types) {
      for (let i = 0; i < dt.types.length; i++) {
        if (dt.types[i] === 'Files') return true
      }
    }
    return false
  }

  const onExternalDragOver = (e: DragEvent, dirPath: string) => {
    if (!hasExternalFiles(e)) return
    e.preventDefault()
    e.stopPropagation()
    try {
      e.dataTransfer.dropEffect = 'copy'
    } catch {
      /* ignore */
    }
    setDropTargetPath(dirPath)
  }

  const onExternalDragLeave = (e: DragEvent, dirPath: string) => {
    e.stopPropagation()
    // Only clear when leaving the current target (not entering a child).
    const related = e.relatedTarget as Node | null
    if (related && (e.currentTarget as HTMLElement).contains(related)) return
    setDropTargetPath((cur) => (cur === dirPath ? null : cur))
  }

  const onExternalDrop = (e: DragEvent, dirPath: string) => {
    if (!hasExternalFiles(e)) return
    e.preventDefault()
    e.stopPropagation()
    setDropTargetPath(null)
    if (!activeRoot) return
    const files = Array.from(e.dataTransfer?.files ?? [])
    for (const file of files) {
      // Skip directory-like entries (webkit relative path empty folders, zero-size no-type).
      if (!file.name || file.name === '.' || file.name === '..') continue
      void uploadCodingFile(activeRoot, dirPath, file, true)
    }
    if (dirPath) {
      setExpanded((prev) => new Set(prev).add(dirPath))
    }
  }

  const onOpenContextMenu = (e: ReactMouseEvent, path: string, isDir: boolean) => {
    e.preventDefault()
    e.stopPropagation()
    setCtxMenu({ x: e.clientX, y: e.clientY, path, isDir })
  }

  const onToggle = (path: string) => {
    if (!activeRoot) return
    setExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(path)) {
        next.delete(path)
      } else {
        next.add(path)
        const k = fileKey(activeRoot, path)
        if (!useKoma.getState().coding.dirs[k]) {
          refreshCodingDir(activeRoot, path)
        }
      }
      return next
    })
  }

  const onOpenFile = (path: string) => {
    if (!activeRoot) return
    openCodingFile(activeRoot, path)
  }

  const onRefresh = (path: string) => {
    if (!activeRoot) return
    refreshCodingDir(activeRoot, path)
  }

  const onDownload = (path: string) => {
    if (!activeRoot) return
    downloadCodingFile(activeRoot, path)
  }

  const onStartCreate = (dirPath: string, item: 'file' | 'dir') => {
    if (!activeRoot) return
    setExpanded((prev) => new Set(prev).add(dirPath))
    const k = fileKey(activeRoot, dirPath)
    if (dirPath && !useKoma.getState().coding.dirs[k]) {
      refreshCodingDir(activeRoot, dirPath)
    }
    setDraft({ kind: 'create', dirPath, item })
  }

  const onStartRename = (path: string) => {
    setDraft({ kind: 'rename', path })
  }

  const onStartDelete = (path: string, isDir: boolean) => {
    setDraft({ kind: 'delete', path, isDir })
  }

  const onCancelDraft = () => setDraft(null)

  const onSubmitCreate = (dirPath: string, item: 'file' | 'dir', name: string) => {
    if (!activeRoot) return
    const cleaned = cleanName(name)
    if (!cleaned) return
    createCodingItem(activeRoot, joinPath(dirPath, cleaned), item)
    setExpanded((prev) => new Set(prev).add(dirPath))
    setDraft(null)
  }

  const onSubmitRename = (path: string, name: string) => {
    if (!activeRoot) return
    const cleaned = cleanName(name)
    if (!cleaned || cleaned === baseName(path)) {
      setDraft(null)
      return
    }
    const newPath = joinPath(parentDirPath(path), cleaned)
    renameCodingItem(activeRoot, path, newPath)
    // Keep expanded UI state coherent for a renamed directory and its children.
    setExpanded((prev) => {
      const next = new Set<string>()
      for (const p of prev) {
        const mapped = remapPath(p, path, newPath)
        next.add(mapped ?? p)
      }
      return next
    })
    setDraft(null)
  }

  const onConfirmDelete = (path: string) => {
    if (!activeRoot) return
    deleteCodingItem(activeRoot, path)
    setExpanded((prev) => {
      const next = new Set<string>()
      for (const p of prev) {
        if (!isPathOrDescendant(p, path)) next.add(p)
      }
      return next
    })
    setDraft(null)
  }

  // No session attached yet — the welcome screen, before any project is opened.
  if (sessionId === null) {
    return (
      <div className="flex h-full flex-col overflow-hidden">
        <Empty>Open a project to use Coding</Empty>
      </div>
    )
  }

  if (roots.length === 0) {
    return <Empty>No workspaces configured. Add paths under Settings → Session → workdir.</Empty>
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-none items-center gap-1 px-2 py-1.5">
        <div className="min-w-0 flex-1" title={activeRoot ?? ''}>
          <Select
            value={activeRoot ?? roots[0] ?? ''}
            options={roots.map((r) => ({ value: r, label: rootLabel(r) }))}
            onChange={(root) => setActiveCodingRoot(root)}
            disabled={!!draft}
          />
        </div>
        {sideTab === 'files' && !draft ? <>
          <IconBtn label="Refresh root" onClick={() => activeRoot && refreshCodingDir(activeRoot, '')}>
            <RefreshCw size={12} />
          </IconBtn>
          <IconBtn label="New file in root" onClick={() => activeRoot && onStartCreate('', 'file')}>
            <FilePlus size={12} />
          </IconBtn>
          <IconBtn label="New folder in root" onClick={() => activeRoot && onStartCreate('', 'dir')}>
            <FolderPlus size={12} />
          </IconBtn>
        </> : null}
      </div>

      <div className="flex-none px-2 pb-1.5">
        <Segmented
          value={sideTab}
          options={[
            { value: 'files', label: 'Files' },
            { value: 'search', label: 'Search' },
          ]}
          onChange={setSideTab}
        />
      </div>

      {sideTab === 'search' ? (
        <div className="min-h-0 flex-1 overflow-hidden">
          <CodingSearchPanel root={activeRoot} />
        </div>
      ) : (
      <div
        className={`min-h-0 flex-1 overflow-y-auto py-1 ${
          dropTargetPath === '' ? 'bg-koma-accent/5' : ''
        }`}
        onDragOver={(e) => onExternalDragOver(e, '')}
        onDrop={(e) => onExternalDrop(e, '')}
        onDragLeave={(e) => onExternalDragLeave(e, '')}
        onContextMenu={(e) => {
          // Empty-area right-click → root context (upload target only; no download).
          if ((e.target as HTMLElement).closest('[data-coding-row]')) return
          onOpenContextMenu(e, '', true)
        }}
      >
        {!activeRoot ? (
          <Empty>Select a workspace root</Empty>
        ) : rootDir?.loading && !rootDir.entries.length && !rootCreate ? (
          <div className="flex items-center gap-2 px-3 py-3 text-[12px] text-koma-dim">
            <BrailleSpinner size={13} />
            <span>Loading…</span>
          </div>
        ) : rootDir?.error ? (
          <div className="px-3 py-3 text-[12px] text-koma-error">{rootDir.error}</div>
        ) : (
          <>
            {rootCreate ? (
              <CreateDraftRow
                depth={0}
                item={rootCreate.item}
                onSubmit={(name) => onSubmitCreate('', rootCreate.item, name)}
                onCancel={onCancelDraft}
              />
            ) : null}
            {rootDir?.entries?.length ? (
              rootDir.entries.map((entry) => (
                <TreeNode
                  key={fileKey(activeRoot, entry.path)}
                  root={activeRoot}
                  entry={entry}
                  depth={0}
                  expanded={expanded}
                  draft={draft}
                  dropTargetPath={dropTargetPath}
                  onToggle={onToggle}
                  onOpenFile={onOpenFile}
                  onRefresh={onRefresh}
                  onStartCreate={onStartCreate}
                  onStartRename={onStartRename}
                  onStartDelete={onStartDelete}
                  onDownload={onDownload}
                  onCancelDraft={onCancelDraft}
                  onSubmitCreate={onSubmitCreate}
                  onSubmitRename={onSubmitRename}
                  onConfirmDelete={onConfirmDelete}
                  onExternalDragOver={onExternalDragOver}
                  onExternalDrop={onExternalDrop}
                  onExternalDragLeave={onExternalDragLeave}
                  onOpenContextMenu={onOpenContextMenu}
                />
              ))
            ) : !rootCreate ? (
              <Empty>
                {dropTargetPath === '' ? 'Drop files to upload' : 'Empty workspace'}
              </Empty>
            ) : null}
          </>
        )}
      </div>
      )}

      {ctxMenu && activeRoot && sideTab === 'files' ? (
        <div
          className="fixed z-[80] min-w-[140px] rounded border border-koma-border bg-koma-panel py-1 shadow-lg"
          style={{ left: ctxMenu.x, top: ctxMenu.y }}
          onClick={(e) => e.stopPropagation()}
          onContextMenu={(e) => e.preventDefault()}
        >
          {!ctxMenu.isDir ? (
            <button
              type="button"
              className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[12px] text-koma-fg opacity-90 hover:bg-koma-hover"
              onClick={() => {
                onDownload(ctxMenu.path)
                setCtxMenu(null)
              }}
            >
              <Download size={12} className="opacity-70" />
              Download
            </button>
          ) : null}
          {ctxMenu.isDir ? (
            <>
              <button
                type="button"
                className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[12px] text-koma-fg opacity-90 hover:bg-koma-hover"
                onClick={() => {
                  onStartCreate(ctxMenu.path, 'file')
                  setCtxMenu(null)
                }}
              >
                <FilePlus size={12} className="opacity-70" />
                New file
              </button>
              <button
                type="button"
                className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[12px] text-koma-fg opacity-90 hover:bg-koma-hover"
                onClick={() => {
                  onStartCreate(ctxMenu.path, 'dir')
                  setCtxMenu(null)
                }}
              >
                <FolderPlus size={12} className="opacity-70" />
                New folder
              </button>
              <button
                type="button"
                className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[12px] text-koma-fg opacity-90 hover:bg-koma-hover"
                onClick={() => {
                  onRefresh(ctxMenu.path)
                  setCtxMenu(null)
                }}
              >
                <RefreshCw size={12} className="opacity-70" />
                Refresh
              </button>
            </>
          ) : null}
          {ctxMenu.path !== '' ? (
            <>
              <div className="my-1 border-t border-koma-border" />
              <button
                type="button"
                className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[12px] text-koma-fg opacity-90 hover:bg-koma-hover"
                onClick={() => {
                  onStartRename(ctxMenu.path)
                  setCtxMenu(null)
                }}
              >
                <Pencil size={12} className="opacity-70" />
                Rename
              </button>
              <button
                type="button"
                className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[12px] text-koma-error opacity-90 hover:bg-koma-error/15"
                onClick={() => {
                  onStartDelete(ctxMenu.path, ctxMenu.isDir)
                  setCtxMenu(null)
                }}
              >
                <Trash2 size={12} className="opacity-70" />
                Delete
              </button>
            </>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}
