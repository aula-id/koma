import { useEffect, useMemo, useState } from 'react'
import {
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
} from 'lucide-react'
import { useKoma } from '../../store/koma'
import { baseName, fileKey, parentDirPath, type FileTreeEntry } from '../../store/coding'
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

type TreeNodeProps = {
  root: string
  entry: FileTreeEntry
  depth: number
  expanded: Set<string>
  onToggle: (path: string) => void
  onOpenFile: (path: string) => void
  onRefresh: (path: string) => void
  onCreate: (dirPath: string, kind: 'file' | 'dir') => void
  onRename: (path: string) => void
  onDelete: (path: string, isDir: boolean) => void
}

function TreeNode({
  root,
  entry,
  depth,
  expanded,
  onToggle,
  onOpenFile,
  onRefresh,
  onCreate,
  onRename,
  onDelete,
}: TreeNodeProps) {
  const key = fileKey(root, entry.path)
  const dirState = useKoma((s) => (entry.isDir ? s.coding.dirs[key] : null))
  const fileState = useKoma((s) => (!entry.isDir ? s.coding.files[key] : null))
  const dirty = !!fileState?.dirty
  const isNew = dirty && fileState?.savedContent === null
  const isOpen = entry.isDir && expanded.has(entry.path)
  const pad = 8 + depth * 12

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
          <span className="min-w-0 truncate">
            {dirty ? (
              <span className={`mr-1 font-mono text-[11px] font-semibold ${isNew ? 'text-koma-success' : 'text-koma-accent'}`}>
                {isNew ? 'A' : 'M'}
              </span>
            ) : null}
            {entry.name}
          </span>
        </button>

        <div className="flex flex-none items-center opacity-0 group-hover:opacity-100">
          {entry.isDir && (
            <>
              <IconBtn label="New file" faded onClick={() => onCreate(entry.path, 'file')}>
                <FilePlus size={12} />
              </IconBtn>
              <IconBtn label="New folder" faded onClick={() => onCreate(entry.path, 'dir')}>
                <FolderPlus size={12} />
              </IconBtn>
              <IconBtn label="Refresh" faded onClick={() => onRefresh(entry.path)}>
                <RefreshCw size={12} />
              </IconBtn>
            </>
          )}
          <IconBtn label="Rename" faded onClick={() => onRename(entry.path)}>
            <Pencil size={12} />
          </IconBtn>
          <IconBtn label="Delete" tone="red" faded onClick={() => onDelete(entry.path, entry.isDir)}>
            <Trash2 size={12} />
          </IconBtn>
        </div>
      </div>

      {entry.isDir && isOpen && (
        <div>
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
                onToggle={onToggle}
                onOpenFile={onOpenFile}
                onRefresh={onRefresh}
                onCreate={onCreate}
                onRename={onRename}
                onDelete={onDelete}
              />
            ))
          ) : dirState && !dirState.loading ? (
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
export function CodingPanel() {
  const workdir = useKoma((s) => s.settingsValues?.workdir ?? [])
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
    if (!activeRoot) return
    refreshCodingDir(activeRoot, '')
    setExpanded(new Set(['']))
  }, [activeRoot, refreshCodingDir])

  const rootKey = activeRoot ? fileKey(activeRoot, '') : null
  const rootDir = rootKey ? dirs[rootKey] : null
  const roots = useMemo(() => workdir.slice(), [workdir])

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

  const onCreate = (dirPath: string, kind: 'file' | 'dir') => {
    if (!activeRoot) return
    const label = kind === 'file' ? 'New file name' : 'New folder name'
    const name = window.prompt(label)
    if (!name || !name.trim()) return
    const cleaned = name.trim().replace(/^\/+|\/+$/g, '')
    if (!cleaned || cleaned.includes('..')) return
    createCodingItem(activeRoot, joinPath(dirPath, cleaned), kind)
    setExpanded((prev) => new Set(prev).add(dirPath))
  }

  const onRename = (path: string) => {
    if (!activeRoot) return
    const current = baseName(path)
    const next = window.prompt('Rename to', current)
    if (!next || !next.trim() || next.trim() === current) return
    const cleaned = next.trim().replace(/^\/+|\/+$/g, '')
    if (!cleaned || cleaned.includes('..') || cleaned.includes('/')) return
    renameCodingItem(activeRoot, path, joinPath(parentDirPath(path), cleaned))
  }

  const onDelete = (path: string, isDir: boolean) => {
    if (!activeRoot) return
    const label = isDir
      ? `Delete folder "${baseName(path)}" and all its contents?`
      : `Delete file "${baseName(path)}"?`
    if (!window.confirm(label)) return
    deleteCodingItem(activeRoot, path)
  }

  if (roots.length === 0) {
    return <Empty>No workspaces configured. Add paths under Settings → Session → workdir.</Empty>
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-none items-center gap-1 border-b border-koma-border px-2 py-1.5">
        <select
          value={activeRoot ?? roots[0] ?? ''}
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
        <IconBtn label="Refresh root" onClick={() => activeRoot && refreshCodingDir(activeRoot, '')}>
          <RefreshCw size={12} />
        </IconBtn>
        <IconBtn label="New file in root" onClick={() => activeRoot && onCreate('', 'file')}>
          <FilePlus size={12} />
        </IconBtn>
        <IconBtn label="New folder in root" onClick={() => activeRoot && onCreate('', 'dir')}>
          <FolderPlus size={12} />
        </IconBtn>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto py-1">
        {!activeRoot ? (
          <Empty>Select a workspace root</Empty>
        ) : rootDir?.loading && !rootDir.entries.length ? (
          <div className="flex items-center gap-2 px-3 py-3 text-[12px] text-koma-dim">
            <BrailleSpinner size={13} />
            <span>Loading…</span>
          </div>
        ) : rootDir?.error ? (
          <div className="px-3 py-3 text-[12px] text-koma-error">{rootDir.error}</div>
        ) : rootDir?.entries?.length ? (
          rootDir.entries.map((entry) => (
            <TreeNode
              key={fileKey(activeRoot, entry.path)}
              root={activeRoot}
              entry={entry}
              depth={0}
              expanded={expanded}
              onToggle={onToggle}
              onOpenFile={onOpenFile}
              onRefresh={onRefresh}
              onCreate={onCreate}
              onRename={onRename}
              onDelete={onDelete}
            />
          ))
        ) : (
          <Empty>Empty workspace</Empty>
        )}
      </div>
    </div>
  )
}
