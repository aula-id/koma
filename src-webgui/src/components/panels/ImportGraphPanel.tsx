import { useEffect, useMemo, useState } from 'react'
import {
  ChevronDown,
  ChevronRight,
  File,
  Folder,
  FolderOpen,
  RefreshCw,
} from 'lucide-react'
import { useKoma } from '../../store/koma'
import { fileKey, type DirState, type FileTreeEntry } from '../../store/coding'
import { BrailleSpinner } from '../BrailleSpinner'
import { Select } from './form'
import { Empty, IconBtn } from './helpers'

import { sourceLanguage, SOURCE_LANGUAGES, type SourceLanguage } from '../../lib/importGraphLanguages'

const ALL_LANGUAGES = '__all_languages__'
const MULTIPLE = '__multiple_filters__'
const EMPTY_ROOTS: string[] = []
type Dirs = Record<string, DirState>

function normalizePath(path: string): string {
  const slashed = path.replace(/\\/g, '/')
  const unc = slashed.startsWith('//')
  const normalized = slashed.replace(/^\/+/, '').replace(/\/{2,}/g, '/')
  const prefixed = unc ? `//${normalized}` : slashed.startsWith('/') ? `/${normalized}` : normalized
  if (prefixed === '/' || prefixed === '//' || /^[A-Za-z]:\/$/.test(prefixed)) return prefixed
  return prefixed.replace(/\/+$/, '')
}

function absolutePath(root: string, relative: string): string {
  const canonicalRoot = normalizePath(root)
  const canonicalRelative = normalizePath(relative).replace(/^\/+/, '')
  if (!canonicalRelative) return canonicalRoot
  return normalizePath(`${canonicalRoot}/${canonicalRelative}`)
}

function pathsEqual(left: string | null, right: string): boolean {
  if (!left) return false
  const a = normalizePath(left)
  const b = normalizePath(right)
  const windows = /^[A-Za-z]:\//.test(a) || /^[A-Za-z]:\//.test(b)
  return windows ? a.toLowerCase() === b.toLowerCase() : a === b
}

function rootLabel(root: string): string {
  const parts = normalizePath(root).split('/').filter(Boolean)
  return parts[parts.length - 1] || root
}

function uniqueRoots(roots: string[]): string[] {
  return roots.filter((root, index) => root.length > 0 && roots.indexOf(root) === index)
}

function entryMatches(
  root: string,
  entry: FileTreeEntry,
  dirs: Dirs,
  query: string,
  languages: string[],
): boolean {
  if (!entry.isDir) {
    const language = sourceLanguage(entry.path)
    if (languages.length > 0 && (!language || !languages.includes(language))) return false
    return !query || entry.path.toLowerCase().includes(query) || entry.name.toLowerCase().includes(query)
  }

  const state = dirs[fileKey(root, entry.path)]
  const descendantsMatch = state?.entries.some((child) => entryMatches(root, child, dirs, query, languages)) ?? false
  const queryMatches = !query
    || entry.path.toLowerCase().includes(query)
    || entry.name.toLowerCase().includes(query)
    || descendantsMatch
  if (!queryMatches) return false
  if (languages.length === 0) return true
  // A lazy folder cannot be ruled out until its contents have been loaded.
  return !state || descendantsMatch
}

function visibleEntries(
  root: string,
  entries: FileTreeEntry[],
  dirs: Dirs,
  query: string,
  languages: string[],
): FileTreeEntry[] {
  return entries.filter((entry) => entryMatches(root, entry, dirs, query, languages))
}

function TreeNode({
  root,
  entry,
  depth,
  dirs,
  expanded,
  query,
  languages,
  selectedPath,
  onToggle,
  onOpenFile,
  onRefresh,
}: {
  root: string
  entry: FileTreeEntry
  depth: number
  dirs: Dirs
  expanded: Set<string>
  query: string
  languages: string[]
  selectedPath: string | null
  onToggle: (path: string) => void
  onOpenFile: (path: string) => void
  onRefresh: (path: string) => void
}) {
  const key = fileKey(root, entry.path)
  const dirState = entry.isDir ? dirs[key] : null
  const isOpen = entry.isDir && expanded.has(key)
  const language = entry.isDir ? null : sourceLanguage(entry.path)
  const absolute = absolutePath(root, entry.path)
  const selected = !!language && pathsEqual(selectedPath, absolute)
  const pad = 8 + depth * 12

  return (
    <div>
      <div
        className={`flex h-7 min-w-0 items-center gap-1 pr-2 text-[12px] hover:bg-koma-hover ${
          selected ? 'bg-koma-accent/10 text-koma-accent' : language || entry.isDir ? 'text-koma-fg' : 'text-koma-dim opacity-60'
        }`}
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
          disabled={!entry.isDir && !language}
          onClick={() => (entry.isDir ? onToggle(entry.path) : onOpenFile(entry.path))}
          className="flex min-w-0 flex-1 items-center gap-1.5 text-left disabled:cursor-not-allowed"
          title={!entry.isDir && !language ? 'No import graph parser for this file type' : absolute}
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
          <span className="min-w-0 flex-1 truncate">{entry.name}</span>
          {language ? (
            <span className="flex-none rounded bg-koma-fg/5 px-1 text-[9px] text-koma-dim opacity-70">
              {language}
            </span>
          ) : null}
        </button>
      </div>

      {entry.isDir && isOpen ? (
        <div>
          {dirState?.loading && !dirState.entries.length ? (
            <div className="flex h-7 items-center gap-2 text-[11px] text-koma-dim" style={{ paddingLeft: pad + 20 }}>
              <BrailleSpinner size={12} />
              <span>Loading…</span>
            </div>
          ) : dirState?.error ? (
            <div className="flex items-center gap-2 px-2 py-1 text-[11px]" style={{ paddingLeft: pad + 20 }}>
              <span className="min-w-0 flex-1 text-koma-error">{dirState.error}</span>
              <button
                type="button"
                onClick={() => onRefresh(entry.path)}
                className="flex-none rounded border border-koma-border bg-koma-bg px-2 py-1 text-koma-fg hover:bg-koma-hover"
              >
                Retry directory
              </button>
            </div>
          ) : dirState ? (
            visibleEntries(root, dirState.entries, dirs, query, languages).map((child) => (
              <TreeNode
                key={fileKey(root, child.path)}
                root={root}
                entry={child}
                depth={depth + 1}
                dirs={dirs}
                expanded={expanded}
                query={query}
                languages={languages}
                selectedPath={selectedPath}
                onToggle={onToggle}
                onOpenFile={onOpenFile}
                onRefresh={onRefresh}
              />
            ))
          ) : null}
          {dirState && !dirState.loading && !dirState.error
            && visibleEntries(root, dirState.entries, dirs, query, languages).length === 0 ? (
              <div className="px-2 py-1 text-[11px] text-koma-dim opacity-60" style={{ paddingLeft: pad + 20 }}>
                Empty
              </div>
            ) : null}
        </div>
      ) : null}
    </div>
  )
}

export function ImportGraphPanel() {
  const availableRoots = useKoma((s) => s.importGraph.availableRoots)
  const filterRoots = useKoma((s) => s.importGraph.filterRoots)
  const filterLanguages = useKoma((s) => s.importGraph.filterLanguages)
  const graphStatus = useKoma((s) => s.importGraph.status)
  const graphLoading = useKoma((s) => s.importGraph.loading)
  const graphError = useKoma((s) => s.importGraph.error)
  const reindexBusy = useKoma((s) => s.importGraph.reindexBusy)
  const edgeCount = useKoma((s) => s.importGraph.edgeCount)
  const selectedPath = useKoma((s) => s.importGraph.selectedPath)
  const dirs = useKoma((s) => s.coding.dirs)
  const req = useKoma((s) => s.req)
  const refreshCodingDir = useKoma((s) => s.refreshCodingDir)
  const openImportGraphTab = useKoma((s) => s.openImportGraphTab)
  const selectImportGraphNode = useKoma((s) => s.selectImportGraphNode)
  const refreshImportGraph = useKoma((s) => s.refreshImportGraph)
  const reindexImportGraph = useKoma((s) => s.reindexImportGraph)
  const setImportGraphRootFilter = useKoma((s) => s.setImportGraphRootFilter)
  const setImportGraphLanguageFilter = useKoma((s) => s.setImportGraphLanguageFilter)
  const [browsingRoot, setBrowsingRoot] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set())

  useEffect(() => {
    req({ r: 'GetSettings' })
    refreshImportGraph()
  }, [req, refreshImportGraph])

  const roots = useMemo(() => {
    // Workspace picker order comes EXCLUSIVELY from backend scoped
    // availableRoots — the Rust linker daemon already scopes to configured
    // workdirs and orders them canonically. Use canonical root for IDs/
    // filters/requests; configuredPath for labels; displayPath for compact.
    return availableRoots.map((r) => r.root)
  }, [availableRoots])
  const validFilters = filterRoots.filter((root) => roots.includes(root))
  const chosenRoot = validFilters.length === 1
    ? validFilters[0]
    : browsingRoot && roots.includes(browsingRoot) ? browsingRoot : roots[0] ?? null

  // Prune stale browsingRoot when settings/session changes reset the roots list.
  // Reset to first configured valid root so the sidebar never shows a
  // root that's no longer in the user's workdir.
  useEffect(() => {
    if (!chosenRoot) {
      setBrowsingRoot(null)
      return
    }
    if (!browsingRoot || !roots.includes(browsingRoot)) setBrowsingRoot(chosenRoot)
    const key = fileKey(chosenRoot, '')
    if (!useKoma.getState().coding.dirs[key]) refreshCodingDir(chosenRoot, '')
  }, [chosenRoot, browsingRoot, roots, refreshCodingDir])

  const rootDir = chosenRoot ? dirs[fileKey(chosenRoot, '')] : null
  const normalizedQuery = query.trim().toLowerCase()
  const entries = chosenRoot && rootDir
    ? visibleEntries(chosenRoot, rootDir.entries, dirs, normalizedQuery, filterLanguages)
    : []
  // Language selector scoped to roots that are both in settings workdirs AND have backend metadata.
  const scopedRootsForLangs = useMemo(() => {
    const backendMap = new Map(availableRoots.map((item) => [item.root, item]))
    return roots.map((r) => backendMap.get(r)).filter((r): r is import('../../store/koma').ImportGraphRootInfo => !!r)
  }, [roots, availableRoots])
  const validLanguageFilters = filterLanguages.filter((language) => {
    const inSourceLangs = SOURCE_LANGUAGES.some((item) => item === language)
    if (!inSourceLangs) return false
    // Only keep languages present in at least one scoped root.
    return scopedRootsForLangs.some((r) => r.languages.some((lc) => lc.name === language))
  })
  const languageValue = validLanguageFilters.length === 0
    ? ALL_LANGUAGES
    : validLanguageFilters.length === 1 ? validLanguageFilters[0] : MULTIPLE

  const onToggle = (path: string) => {
    if (!chosenRoot) return
    const key = fileKey(chosenRoot, path)
    setExpanded((previous) => {
      const next = new Set(previous)
      if (next.has(key)) {
        next.delete(key)
      } else {
        next.add(key)
        if (!useKoma.getState().coding.dirs[key]) refreshCodingDir(chosenRoot, path)
      }
      return next
    })
  }

  const onOpenFile = (path: string) => {
    if (!chosenRoot || !sourceLanguage(path)) return
    openImportGraphTab()
    selectImportGraphNode(absolutePath(chosenRoot, path))
  }

  const onRefresh = () => {
    if (chosenRoot) refreshCodingDir(chosenRoot, '')
    reindexImportGraph()
  }

  if (roots.length === 0) {
    return <Empty>No workspaces configured. Add paths under Settings → Session → workdir.</Empty>
  }

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden bg-koma-panel">
      <div className="flex flex-none items-center gap-1 px-2 py-1.5">
        {roots.length > 1 ? (
          <div className="min-w-0 flex-1" title={chosenRoot ?? ''}>
            <Select
              value={chosenRoot ?? ''}
              options={roots.map((root) => {
                const backend = availableRoots.find((r) => r.root === root)
                const label = backend?.displayPath ?? rootLabel(root)
                return { value: root, label }
              })}
              onChange={(root) => {
                setBrowsingRoot(root)
                setImportGraphRootFilter([root])
              }}
            />
          </div>
        ) : (
          <div
            className="flex h-7 min-w-0 flex-1 items-center rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg opacity-60"
            title={roots[0]}
          >
            <span className="truncate">{availableRoots.find((r) => r.root === roots[0])?.displayPath ?? rootLabel(roots[0])}</span>
          </div>
        )}
        <button
          type="button"
          onClick={onRefresh}
          disabled={reindexBusy}
          title="Reindex configured workspaces"
          aria-label="Reindex configured workspaces"
          className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100 disabled:cursor-default disabled:opacity-40"
        >
          <RefreshCw size={12} className={reindexBusy ? 'animate-spin' : ''} />
        </button>
      </div>

      <div className="flex-none px-2 pb-1.5">
        <Select
          value={languageValue}
          options={[
            { value: ALL_LANGUAGES, label: 'All languages' },
            ...(languageValue === MULTIPLE ? [{ value: MULTIPLE, label: `${validLanguageFilters.length} languages` }] : []),
            ...Array.from(new Set(scopedRootsForLangs.flatMap((r) => r.languages.map((lc) => lc.name)))).sort().map((language) => ({ value: language, label: language })),
          ]}
          onChange={(language) => {
            if (language === MULTIPLE) return
            setImportGraphLanguageFilter(language === ALL_LANGUAGES ? [] : [language])
          }}
        />
      </div>

      <div className="flex-none px-2 pb-1.5">
        <input
          type="text"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Filter files…"
          className="h-7 w-full rounded border border-koma-border bg-koma-bg px-2 text-[11px] text-koma-fg outline-none placeholder:text-koma-dim/50 focus:border-koma-accent"
        />
      </div>

      {graphStatus === 'scanning' ? (
        <div className="flex flex-none items-center gap-2 border-y border-koma-accent/15 px-3 py-1.5 text-[10px] text-koma-dim">
          <BrailleSpinner size={10} />
          <span>Indexing import graph…</span>
        </div>
      ) : graphError ? (
        <div className="flex flex-none items-center gap-2 border-y border-koma-error/15 px-3 py-1.5 text-[10px]">
          <span className="min-w-0 flex-1 truncate text-koma-error" title={graphError}>Graph unavailable: {graphError}</span>
          <button type="button" onClick={() => reindexImportGraph()} className="flex-none text-koma-fg hover:text-koma-accent">
            Retry
          </button>
        </div>
      ) : null}

      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto py-1">
        <div className="flex h-6 flex-none items-center px-3 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">
          <span>Files</span>
          {edgeCount > 0 ? <span className="ml-auto normal-case tracking-normal">{edgeCount} edges</span> : null}
          {graphLoading ? (
            <span className="ml-2 flex items-center gap-1 normal-case tracking-normal">
              <BrailleSpinner size={10} /> Updating graph…
            </span>
          ) : null}
        </div>

        {rootDir?.loading && !rootDir.entries.length ? (
          <div className="flex items-center gap-2 px-3 py-3 text-[12px] text-koma-dim">
            <BrailleSpinner size={13} />
            <span>Loading filesystem…</span>
          </div>
        ) : rootDir?.error ? (
          <div className="flex items-center gap-2 px-3 py-3 text-[11px]">
            <span className="min-w-0 flex-1 text-koma-error">{rootDir.error}</span>
            <button
              type="button"
              onClick={() => chosenRoot && refreshCodingDir(chosenRoot, '')}
              className="flex-none rounded border border-koma-border bg-koma-bg px-2 py-1 text-koma-fg hover:bg-koma-hover"
            >
              Retry directory
            </button>
          </div>
        ) : entries.length > 0 && chosenRoot ? (
          entries.map((entry) => (
            <TreeNode
              key={fileKey(chosenRoot, entry.path)}
              root={chosenRoot}
              entry={entry}
              depth={0}
              dirs={dirs}
              expanded={expanded}
              query={normalizedQuery}
              languages={filterLanguages}
              selectedPath={selectedPath}
              onToggle={onToggle}
              onOpenFile={onOpenFile}
              onRefresh={(path) => refreshCodingDir(chosenRoot, path)}
            />
          ))
        ) : rootDir ? (
          <Empty>{rootDir.entries.length > 0 ? 'No files match the current filters.' : 'Empty workspace'}</Empty>
        ) : null}
      </div>
    </div>
  )
}
