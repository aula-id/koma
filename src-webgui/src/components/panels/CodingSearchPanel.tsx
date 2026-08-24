import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import {
  CaseSensitive,
  ChevronDown,
  ChevronRight,
  File,
  Regex,
  ReplaceAll,
  WholeWord,
} from 'lucide-react'
import { useKoma } from '../../store/koma'
import { baseName, type ContentSearchFileHit } from '../../store/coding'
import { BrailleSpinner } from '../BrailleSpinner'
import { Empty } from './helpers'

/** Fixed rail width — chevron column; find/replace share one left edge. */
const RAIL = 'w-5'

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
      className={`flex h-6 w-6 flex-none items-center justify-center rounded-sm transition-colors ${
        on
          ? 'bg-koma-accent/18 text-koma-accent'
          : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
      }`}
    >
      {children}
    </button>
  )
}

/** VS Code-like bordered field: input + optional trailing action cluster. */
function FieldRow({
  children,
  trailing,
}: {
  children: ReactNode
  trailing?: ReactNode
}) {
  return (
    <div className="flex h-7 min-w-0 items-stretch rounded-[3px] border border-koma-border bg-koma-bg focus-within:border-koma-fg/40">
      <div className="flex min-w-0 flex-1 items-center">{children}</div>
      {trailing ? (
        <div className="flex flex-none items-center gap-px border-l border-koma-border/80 px-0.5">
          {trailing}
        </div>
      ) : null}
    </div>
  )
}

function fieldInputClass() {
  return 'h-full w-full min-w-0 bg-transparent px-2 font-mono text-[12px] leading-none text-koma-fg outline-none placeholder:text-koma-dim placeholder:opacity-50'
}

function FilterLabel({ children }: { children: ReactNode }) {
  return (
    <div className="select-none px-0.5 pt-0.5 text-[11px] leading-none text-koma-dim opacity-80">
      {children}
    </div>
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

  useEffect(() => {
    setExpandedFiles(new Set(search.results.map((r) => r.path)))
  }, [search.results])

  useEffect(() => {
    setIncludeLocal(search.includeGlob)
  }, [search.includeGlob])
  useEffect(() => {
    setExcludeLocal(search.excludeGlob)
  }, [search.excludeGlob])

  useEffect(() => {
    const t = window.setTimeout(() => searchInputRef.current?.focus(), 0)
    return () => window.clearTimeout(t)
  }, [])

  const totalMatches = useMemo(
    () => search.results.reduce((n, f) => n + f.matches.length, 0),
    [search.results],
  )

  const commitGlobs = () => {
    if (includeLocal === search.includeGlob && excludeLocal === search.excludeGlob) return
    setGlobs(includeLocal, excludeLocal)
  }

  const canReplace = !!root && !!search.query.trim() && !search.replacing

  if (!root) {
    return <Empty>Select a workspace root</Empty>
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/*
        VS Code Search symmetry:
          [>] [ find …………… | Aa ab .* ]
              [ replace ……… | ⇄      ]
          files to include
          [ …………………………… ]
          files to exclude
          [ …………………………… ]
        Chevron is a fixed rail; find/replace/filters share one column width.
      */}
      <div className="flex flex-none flex-col gap-1.5 border-b border-koma-border px-2 py-2">
        <div className="flex min-w-0 items-start gap-1">
          <button
            type="button"
            title={search.replaceOpen ? 'Hide replace' : 'Toggle Replace mode'}
            aria-label={search.replaceOpen ? 'Hide replace' : 'Show replace'}
            aria-expanded={search.replaceOpen}
            onClick={() => setFlag('replaceOpen', !search.replaceOpen)}
            className={`${RAIL} mt-0.5 flex h-7 flex-none items-center justify-center rounded-sm text-koma-dim hover:bg-koma-hover hover:text-koma-fg`}
          >
            {search.replaceOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          </button>

          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <FieldRow
              trailing={
                <>
                  <ToggleChip
                    on={search.caseSensitive}
                    title="Match Case (Alt+C)"
                    onClick={() => setFlag('caseSensitive', !search.caseSensitive)}
                  >
                    <CaseSensitive size={14} />
                  </ToggleChip>
                  <ToggleChip
                    on={search.wholeWord}
                    title="Match Whole Word (Alt+W)"
                    onClick={() => setFlag('wholeWord', !search.wholeWord)}
                  >
                    <WholeWord size={14} />
                  </ToggleChip>
                  <ToggleChip
                    on={search.isRegex}
                    title="Use Regular Expression (Alt+R)"
                    onClick={() => setFlag('isRegex', !search.isRegex)}
                  >
                    <Regex size={14} />
                  </ToggleChip>
                </>
              }
            >
              <input
                ref={searchInputRef}
                value={search.query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search"
                spellCheck={false}
                className={fieldInputClass()}
              />
            </FieldRow>

            {search.replaceOpen ? (
              <FieldRow
                trailing={
                  <ToggleChip
                    on={false}
                    title="Replace All (Ctrl+Enter)"
                    onClick={() => {
                      if (!canReplace) return
                      replaceAll(root)
                    }}
                  >
                    {search.replacing ? (
                      <BrailleSpinner size={13} />
                    ) : (
                      <ReplaceAll size={14} className="text-koma-fg opacity-80" />
                    )}
                  </ToggleChip>
                }
              >
                <input
                  value={search.replace}
                  onChange={(e) => setReplace(e.target.value)}
                  placeholder="Replace"
                  spellCheck={false}
                  className={fieldInputClass()}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
                      e.preventDefault()
                      if (canReplace) replaceAll(root)
                    }
                  }}
                />
              </FieldRow>
            ) : null}
          </div>
        </div>

        {/* Filters: same column as find/replace (rail spacer keeps left edge). */}
        <div className="flex min-w-0 items-start gap-1">
          <div className={`${RAIL} flex-none`} aria-hidden />
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <FilterLabel>files to include</FilterLabel>
            <FieldRow>
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
                placeholder="e.g. *.ts, src/**"
                spellCheck={false}
                className={fieldInputClass()}
              />
            </FieldRow>
            <FilterLabel>files to exclude</FilterLabel>
            <FieldRow>
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
                placeholder="e.g. **/node_modules/**"
                spellCheck={false}
                className={fieldInputClass()}
              />
            </FieldRow>
          </div>
        </div>

        <div className="flex min-h-[14px] min-w-0 items-center gap-1.5">
          <div className={`${RAIL} flex-none`} aria-hidden />
          <div className="flex min-w-0 flex-1 items-center gap-1.5 text-[10px] text-koma-dim">
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
                {totalMatches} result{totalMatches === 1 ? '' : 's'} in {search.results.length}{' '}
                file{search.results.length === 1 ? '' : 's'}
                {search.truncated ? ' (truncated)' : ''}
              </span>
            ) : (
              <span className="opacity-50">Type to search workspace</span>
            )}
          </div>
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
