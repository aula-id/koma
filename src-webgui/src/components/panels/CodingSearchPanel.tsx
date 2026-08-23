import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import {
  CaseSensitive,
  ChevronDown,
  ChevronRight,
  File,
  Regex,
  Replace,
  WholeWord,
} from 'lucide-react'
import { useKoma } from '../../store/koma'
import { baseName, type ContentSearchFileHit } from '../../store/coding'
import { BrailleSpinner } from '../BrailleSpinner'
import { Empty, IconBtn } from './helpers'

function ToggleChip({
  on,
  title,
  onClick,
  children,
}: {
  on: boolean
  title: string
  onClick: () => void
  children: ReactNode
}) {
  return (
    <button
      type="button"
      title={title}
      aria-pressed={on}
      onClick={onClick}
      className={`flex h-5 w-5 flex-none items-center justify-center rounded border text-[11px] transition-colors ${
        on
          ? 'border-koma-accent bg-koma-accent/15 text-koma-fg opacity-100'
          : 'border-transparent text-koma-fg opacity-50 hover:bg-koma-hover hover:opacity-90'
      }`}
    >
      {children}
    </button>
  )
}

function SearchHitGroup({
  hit,
  expanded,
  onToggle,
  onOpen,
}: {
  hit: ContentSearchFileHit
  expanded: boolean
  onToggle: () => void
  onOpen: (path: string, line: number, col: number) => void
}) {
  return (
    <div className="select-none">
      <button
        type="button"
        className="flex w-full min-w-0 items-center gap-1 px-2 py-0.5 text-left text-[12px] text-koma-fg hover:bg-koma-hover"
        onClick={onToggle}
        title={hit.path}
      >
        {expanded ? (
          <ChevronDown size={12} className="flex-none opacity-60" />
        ) : (
          <ChevronRight size={12} className="flex-none opacity-60" />
        )}
        <File size={12} className="flex-none opacity-60" />
        <span className="min-w-0 flex-1 truncate font-mono opacity-90">{baseName(hit.path)}</span>
        <span className="flex-none text-[10px] text-koma-dim tabular-nums opacity-70">
          {hit.matches.length}
        </span>
      </button>
      {expanded ? (
        <div className="pb-0.5">
          {hit.path.includes('/') ? (
            <div className="truncate px-2 pl-7 text-[10px] text-koma-dim opacity-60" title={hit.path}>
              {hit.path}
            </div>
          ) : null}
          {hit.matches.map((m, i) => (
            <button
              key={`${hit.path}:${m.line}:${m.col}:${i}`}
              type="button"
              className="flex w-full min-w-0 items-start gap-1.5 px-2 py-0.5 pl-7 text-left font-mono text-[11px] text-koma-fg hover:bg-koma-hover"
              onClick={() => onOpen(hit.path, m.line, m.col || 1)}
              title={`${hit.path}:${m.line}`}
            >
              <span className="w-7 flex-none text-right text-[10px] text-koma-dim tabular-nums opacity-70">
                {m.line}
              </span>
              <span className="min-w-0 flex-1 truncate opacity-85">{m.text.trimStart()}</span>
            </button>
          ))}
        </div>
      ) : null}
    </div>
  )
}

/** VS Code-style content search pane for the Coding sidepanel. */
export function CodingSearchPanel({ root }: { root: string | null }) {
  const search = useKoma((s) => s.coding.search)
  const setQuery = useKoma((s) => s.setCodingSearchQuery)
  const setReplace = useKoma((s) => s.setCodingSearchReplace)
  const setFlag = useKoma((s) => s.setCodingSearchFlag)
  const setGlobs = useKoma((s) => s.setCodingSearchGlobs)
  const runSearch = useKoma((s) => s.searchCodingContent)
  const replaceAll = useKoma((s) => s.replaceCodingContentAll)
  const openHit = useKoma((s) => s.openCodingSearchHit)

  const [includeLocal, setIncludeLocal] = useState(search.includeGlob)
  const [excludeLocal, setExcludeLocal] = useState(search.excludeGlob)
  const [expandedFiles, setExpandedFiles] = useState<Set<string>>(() => new Set())
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const searchInputRef = useRef<HTMLInputElement | null>(null)

  // Debounced search on query / flags / globs / root.
  useEffect(() => {
    if (!root) return
    if (debounceRef.current) clearTimeout(debounceRef.current)
    debounceRef.current = setTimeout(() => {
      runSearch(root)
    }, 300)
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current)
    }
  }, [
    root,
    search.query,
    search.caseSensitive,
    search.wholeWord,
    search.isRegex,
    search.includeGlob,
    search.excludeGlob,
    runSearch,
  ])

  // Expand all result files by default when a new result set lands.
  useEffect(() => {
    setExpandedFiles(new Set(search.results.map((r) => r.path)))
  }, [search.results])

  // Keep local glob inputs in sync when store is reset externally.
  useEffect(() => {
    setIncludeLocal(search.includeGlob)
  }, [search.includeGlob])
  useEffect(() => {
    setExcludeLocal(search.excludeGlob)
  }, [search.excludeGlob])

  const totalMatches = useMemo(
    () => search.results.reduce((n, f) => n + f.matches.length, 0),
    [search.results],
  )

  const commitGlobs = () => {
    if (includeLocal === search.includeGlob && excludeLocal === search.excludeGlob) return
    setGlobs(includeLocal, excludeLocal)
  }

  if (!root) {
    return <Empty>Select a workspace root</Empty>
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-none flex-col gap-1 border-b border-koma-border px-2 py-1.5">
        {/* Search row */}
        <div className="flex items-center gap-0.5">
          <IconBtn
            label={search.replaceOpen ? 'Hide replace' : 'Show replace'}
            onClick={() => setFlag('replaceOpen', !search.replaceOpen)}
          >
            {search.replaceOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
          </IconBtn>
          <div className="flex min-w-0 flex-1 items-center gap-0.5 rounded border border-koma-border bg-koma-bg px-1">
            <input
              ref={searchInputRef}
              value={search.query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search"
              spellCheck={false}
              className="h-6 min-w-0 flex-1 bg-transparent px-1 font-mono text-[12px] text-koma-fg outline-none placeholder:text-koma-dim placeholder:opacity-60"
            />
            <ToggleChip
              on={search.caseSensitive}
              title="Match Case"
              onClick={() => setFlag('caseSensitive', !search.caseSensitive)}
            >
              <CaseSensitive size={12} />
            </ToggleChip>
            <ToggleChip
              on={search.wholeWord}
              title="Match Whole Word"
              onClick={() => setFlag('wholeWord', !search.wholeWord)}
            >
              <WholeWord size={12} />
            </ToggleChip>
            <ToggleChip
              on={search.isRegex}
              title="Use Regular Expression"
              onClick={() => setFlag('isRegex', !search.isRegex)}
            >
              <Regex size={12} />
            </ToggleChip>
          </div>
        </div>

        {/* Replace row */}
        {search.replaceOpen ? (
          <div className="flex items-center gap-0.5 pl-[22px]">
            <div className="flex min-w-0 flex-1 items-center gap-0.5 rounded border border-koma-border bg-koma-bg px-1">
              <input
                value={search.replace}
                onChange={(e) => setReplace(e.target.value)}
                placeholder="Replace"
                spellCheck={false}
                className="h-6 min-w-0 flex-1 bg-transparent px-1 font-mono text-[12px] text-koma-fg outline-none placeholder:text-koma-dim placeholder:opacity-60"
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
                    e.preventDefault()
                    replaceAll(root)
                  }
                }}
              />
            </div>
            <IconBtn
              label="Replace All"
              tone="emerald"
              onClick={() => {
                if (search.replacing || !search.query.trim()) return
                replaceAll(root)
              }}
            >
              {search.replacing ? <BrailleSpinner size={12} /> : <Replace size={12} />}
            </IconBtn>
          </div>
        ) : null}

        {/* files to include/exclude */}
        <button
          type="button"
          className="flex items-center gap-1 px-0.5 text-left text-[10px] uppercase tracking-wide text-koma-dim opacity-70 hover:opacity-100"
          onClick={() => setFlag('filtersOpen', !search.filtersOpen)}
        >
          {search.filtersOpen ? <ChevronDown size={10} /> : <ChevronRight size={10} />}
          files to include / exclude
        </button>
        {search.filtersOpen ? (
          <div className="flex flex-col gap-1 pl-[14px]">
            <input
              value={includeLocal}
              onChange={(e) => setIncludeLocal(e.target.value)}
              onBlur={commitGlobs}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault()
                  commitGlobs()
                }
              }}
              placeholder="files to include (e.g. *.ts, src/**)"
              spellCheck={false}
              className="h-6 rounded border border-koma-border bg-koma-bg px-1.5 font-mono text-[11px] text-koma-fg outline-none placeholder:text-koma-dim placeholder:opacity-55 focus:border-koma-fg/40"
            />
            <input
              value={excludeLocal}
              onChange={(e) => setExcludeLocal(e.target.value)}
              onBlur={commitGlobs}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault()
                  commitGlobs()
                }
              }}
              placeholder="files to exclude"
              spellCheck={false}
              className="h-6 rounded border border-koma-border bg-koma-bg px-1.5 font-mono text-[11px] text-koma-fg outline-none placeholder:text-koma-dim placeholder:opacity-55 focus:border-koma-fg/40"
            />
          </div>
        ) : null}

        {/* status line */}
        <div className="flex min-h-[14px] items-center gap-1.5 px-0.5 text-[10px] text-koma-dim">
          {search.loading ? (
            <>
              <BrailleSpinner size={10} />
              <span>Searching…</span>
            </>
          ) : search.error ? (
            <span className="text-koma-error">{search.error}</span>
          ) : search.replaceError ? (
            <span className="text-koma-error">{search.replaceError}</span>
          ) : search.lastReplaceSummary ? (
            <span className="text-koma-fg opacity-70">{search.lastReplaceSummary}</span>
          ) : search.query.trim() ? (
            <span>
              {totalMatches} result{totalMatches === 1 ? '' : 's'} in {search.results.length} file
              {search.results.length === 1 ? '' : 's'}
              {search.truncated ? ' (truncated)' : ''}
            </span>
          ) : (
            <span className="opacity-50">Type to search workspace</span>
          )}
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto py-1">
        {!search.query.trim() ? (
          <Empty>Enter a search term</Empty>
        ) : search.loading && search.results.length === 0 ? (
          <div className="flex items-center gap-2 px-3 py-3 text-[12px] text-koma-dim">
            <BrailleSpinner size={13} />
            <span>Searching…</span>
          </div>
        ) : search.error ? (
          <div className="px-3 py-3 text-[12px] text-koma-error">{search.error}</div>
        ) : search.results.length === 0 ? (
          <Empty>No results</Empty>
        ) : (
          search.results.map((hit) => (
            <SearchHitGroup
              key={hit.path}
              hit={hit}
              expanded={expandedFiles.has(hit.path)}
              onToggle={() => {
                setExpandedFiles((prev) => {
                  const next = new Set(prev)
                  if (next.has(hit.path)) next.delete(hit.path)
                  else next.add(hit.path)
                  return next
                })
              }}
              onOpen={(path, line, col) => openHit(root, path, line, col)}
            />
          ))
        )}
      </div>
    </div>
  )
}
