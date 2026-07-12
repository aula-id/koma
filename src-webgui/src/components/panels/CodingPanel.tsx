import { useEffect, useMemo, useRef, useState, type FormEvent, type KeyboardEvent } from 'react'
import {
  Check,
  ChevronDown,
  ChevronRight,
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
  onToggle: (path: string) => void
  onOpenFile: (path: string) => void
  onRefresh: (path: string) => void
  onStartCreate: (dirPath: string, item: 'file' | 'dir') => void
  onStartRename: (path: string) => void
  onStartDelete: (path: string, isDir: boolean) => void
  onCancelDraft: () => void
  onSubmitCreate: (dirPath: string, item: 'file' | 'dir', name: string) => void
  onSubmitRename: (path: string, name: string) => void
  onConfirmDelete: (path: string) => void
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
  onToggle,
  onOpenFile,
  onRefresh,
  onStartCreate,
  onStartRename,
  onStartDelete,
  onCancelDraft,
  onSubmitCreate,
  onSubmitRename,
  onConfirmDelete,
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
        className="group flex h-7 min-w-0 items-center gap-1 pr-1 text-[12px] text-koma-fg hover:bg-koma-hover"
        style={{ paddingLeft: pad }}
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
              className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
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
              <span className="min-w-0 truncate">{entry.name}</span>
            </button>

            {!draft ? <div className="flex flex-none items-center opacity-0 group-hover:opacity-100">
              {entry.isDir && (
                <>
                  <IconBtn label="New file" faded onClick={() => onStartCreate(entry.path, 'file')}>
                    <FilePlus size={12} />
                  </IconBtn>
                  <IconBtn label="New folder" faded onClick={() => onStartCreate(entry.path, 'dir')}>
                    <FolderPlus size={12} />
                  </IconBtn>
                  <IconBtn label="Refresh" faded onClick={() => onRefresh(entry.path)}>
                    <RefreshCw size={12} />
                  </IconBtn>
                </>
              )}
              <IconBtn label="Rename" faded onClick={() => onStartRename(entry.path)}>
                <Pencil size={12} />
              </IconBtn>
              <IconBtn
                label="Delete"
                tone="red"
                faded
                onClick={() => onStartDelete(entry.path, entry.isDir)}
              >
                <Trash2 size={12} />
              </IconBtn>
            </div> : null}

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
                onToggle={onToggle}
                onOpenFile={onOpenFile}
                onRefresh={onRefresh}
                onStartCreate={onStartCreate}
                onStartRename={onStartRename}
                onStartDelete={onStartDelete}
                onCancelDraft={onCancelDraft}
                onSubmitCreate={onSubmitCreate}
                onSubmitRename={onSubmitRename}
                onConfirmDelete={onConfirmDelete}
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
  const req = useKoma((s) => s.req)

  const [expanded, setExpanded] = useState<Set<string>>(() => new Set(['']))
  const [draft, setDraft] = useState<Draft | null>(null)

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
    if (!activeRoot) return
    refreshCodingDir(activeRoot, '')
    setExpanded(new Set(['']))
    setDraft(null)
  }, [activeRoot, refreshCodingDir])

  const rootKey = activeRoot ? fileKey(activeRoot, '') : null
  const rootDir = rootKey ? dirs[rootKey] : null
  const roots = useMemo(() => workdir.slice(), [workdir])
  const rootCreate =
    draft?.kind === 'create' && draft.dirPath === '' ? draft : null

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
        <select
          value={activeRoot ?? roots[0] ?? ''}
          disabled={!!draft}
          onChange={(e) => setActiveCodingRoot(e.target.value)}
          className="min-w-0 flex-1 truncate rounded border border-koma-border bg-koma-bg px-1.5 py-1 text-[11px] text-koma-fg outline-none focus:border-koma-fg/40"
          title={activeRoot ?? ''}
        >
          {roots.map((r) => (
            <option key={r} value={r}>
              {rootLabel(r)}
            </option>
          ))}
        </select>
        {!draft ? <>
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

      <div className="min-h-0 flex-1 overflow-y-auto py-1">
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
                  onToggle={onToggle}
                  onOpenFile={onOpenFile}
                  onRefresh={onRefresh}
                  onStartCreate={onStartCreate}
                  onStartRename={onStartRename}
                  onStartDelete={onStartDelete}
                  onCancelDraft={onCancelDraft}
                  onSubmitCreate={onSubmitCreate}
                  onSubmitRename={onSubmitRename}
                  onConfirmDelete={onConfirmDelete}
                />
              ))
            ) : !rootCreate ? (
              <Empty>Empty workspace</Empty>
            ) : null}
          </>
        )}
      </div>
    </div>
  )
}
