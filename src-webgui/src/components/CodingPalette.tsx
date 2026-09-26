import { useEffect, useRef, useState } from 'react'
import { File, Search, Terminal, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import { codingRequest } from '../lib/coding-service'
import { BrailleSpinner } from './BrailleSpinner'
import { showCodingHistory } from './CodingHistory'

type Command = { id: string; title: string; shortcut?: string; editor?: boolean }
const commands: Command[] = [
  { id: 'files', title: 'Go to File', shortcut: 'Ctrl/Cmd+P' },
  { id: 'editor.action.quickOutline', title: 'Go to Symbol in Editor', shortcut: 'Ctrl/Cmd+Shift+O', editor: true },
  { id: 'editor.action.gotoLine', title: 'Go to Line', shortcut: 'Ctrl/Cmd+G', editor: true },
  { id: 'editor.action.formatDocument', title: 'Format Document', shortcut: 'Shift+Alt+F', editor: true },
  { id: 'editor.action.formatSelection', title: 'Format Selection', editor: true },
  { id: 'editor.action.goToImplementation', title: 'Go to Implementation', editor: true },
  { id: 'editor.action.goToTypeDefinition', title: 'Go to Type Definition', editor: true },
  { id: 'koma.rename', title: 'Rename Symbol…', shortcut: 'F2', editor: true },
  { id: 'editor.action.quickFix', title: 'Quick Fix / Code Actions', shortcut: 'Ctrl/Cmd+.', editor: true },
  { id: 'koma.undoWorkspaceEdit', title: 'Undo Workspace Edit', editor: true },
  { id: 'save', title: 'Save File', shortcut: 'Ctrl/Cmd+S', editor: true },
  { id: 'editor.action.revealDefinition', title: 'Go to Definition', shortcut: 'F12', editor: true },
  { id: 'editor.action.referenceSearch.trigger', title: 'Find References', shortcut: 'Shift+F12', editor: true },
  { id: 'editor.action.triggerSuggest', title: 'Trigger Suggestions', editor: true },
  { id: 'editor.action.commentLine', title: 'Toggle Line Comment', editor: true },
  { id: 'editor.action.toggleWordWrap', title: 'Toggle Word Wrap', editor: true },
  { id: 'saveAll', title: 'Save All Open Files' },
  { id: 'recovery', title: 'Recover Unsaved Files' },
  { id: 'history', title: 'Show Local History', editor: true },
  { id: 'terminal', title: 'Open Terminal' },
  { id: 'settings', title: 'Open Settings' },
]
type Hit = { root: string; path: string }

export function showCodingPalette(mode: 'files' | 'commands' = 'files') {
  window.dispatchEvent(new CustomEvent('koma-coding-palette', { detail: mode }))
}

/** Uses the existing compact Koma overlay language; leaves the workspace picker intact. */
export function CodingPalette() {
  const [mode, setMode] = useState<'files' | 'commands' | null>(null)
  const [query, setQuery] = useState('')
  const [scope, setScope] = useState('')
  const [hits, setHits] = useState<Hit[]>([])
  const [selected, setSelected] = useState(0)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [truncated, setTruncated] = useState(false)
  const input = useRef<HTMLInputElement>(null)
  const restoreFocus = useRef<HTMLElement | null>(null)
  const roots = useKoma(s => s.settingsValues?.workdir)
  const activeRoot = useKoma(s => s.coding.activeRoot)
  const hostId = useKoma(s => s.remoteState.hostId ?? 'local')
  const activeTab = useKoma(s => s.ui.tabs.find(t => t.id === s.ui.activeTabId))
  const rootsKey = JSON.stringify(roots ?? [])
  const recent = useKoma(s => s.ui.tabs)

  const close = () => {
    setMode(null)
    requestAnimationFrame(() => restoreFocus.current?.focus())
  }
  useEffect(() => {
    const open = (next: 'files' | 'commands') => {
      restoreFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
      setMode(next); setQuery(''); setScope(useKoma.getState().coding.activeRoot ?? useKoma.getState().settingsValues?.workdir?.[0] ?? '')
      setSelected(0); setError(null)
    }
    const onEvent = (event: Event) => open((event as CustomEvent).detail === 'commands' ? 'commands' : 'files')
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.altKey || event.key.toLowerCase() !== 'p' || event.isComposing) return
      event.preventDefault(); event.stopPropagation()
      open(event.shiftKey ? 'commands' : 'files')
    }
    window.addEventListener('koma-coding-palette', onEvent)
    window.addEventListener('keydown', onKey, true)
    return () => { window.removeEventListener('koma-coding-palette', onEvent); window.removeEventListener('keydown', onKey, true) }
  }, [])
  useEffect(() => { setMode(null) }, [hostId])
  useEffect(() => { if (mode) input.current?.focus() }, [mode])

  useEffect(() => {
    if (mode !== 'files') return
    const controller = new AbortController()
    setSelected(0); setError(null); setTruncated(false)
    const selectedRoots: string[] = scope === '*' ? JSON.parse(rootsKey) : scope ? [scope] : []
    if (!selectedRoots.length) { setHits([]); setBusy(false); return }
    if (!query) {
      const existing = recent.filter((t): t is Extract<typeof t, { kind: 'codingFile' }> => t.kind === 'codingFile' && !t.preview && selectedRoots.includes(t.root))
      setHits(existing.map(t => ({ root: t.root, path: t.path })))
    } else setHits([])
    setBusy(true)
    const timer = setTimeout(() => {
      void Promise.all(selectedRoots.map(async root => {
        const result = await codingRequest<{ paths: string[]; truncated: boolean }>({ hostId, root }, { op: 'paths', query }, controller.signal)
        return { hits: result.paths.map(path => ({ root, path })), truncated: result.truncated }
      })).then(results => {
        if (controller.signal.aborted) return
        const found = results.flatMap(r => r.hits)
        const opened = recent.filter((t): t is Extract<typeof t, { kind: 'codingFile' }> => t.kind === 'codingFile' && !t.preview && selectedRoots.includes(t.root))
        const first = query ? [] : opened.map(t => ({ root: t.root, path: t.path }))
        const unique = new Map([...first, ...found].map(h => [JSON.stringify([h.root, h.path]), h]))
        setHits([...unique.values()].slice(0, 200))
        setTruncated(results.some(r => r.truncated) || unique.size > 200)
      }).catch(e => { if (!controller.signal.aborted) setError(String(e.message ?? e)) })
        .finally(() => { if (!controller.signal.aborted) setBusy(false) })
    }, 180)
    return () => { clearTimeout(timer); controller.abort() }
  }, [mode, query, scope, rootsKey, hostId])

  if (!mode) return null
  const choices = commands.filter(c => (!c.editor || activeTab?.kind === 'codingFile') && c.title.toLowerCase().includes(query.toLowerCase()))
  const count = mode === 'files' ? hits.length : choices.length
  const pick = (index: number) => {
    if (mode === 'files') {
      const hit = hits[index]
      if (!hit) return
      useKoma.getState().openCodingFile(hit.root, hit.path)
      setMode(null)
      return
    }
    const command = choices[index]
    if (!command) return
    if (command.id === 'files') { setMode('files'); setQuery(''); return }
    setMode(null)
    const store = useKoma.getState()
    if (command.id === 'saveAll') { for (const tab of store.ui.tabs) if (tab.kind === 'codingFile' && !tab.preview) store.saveCodingFile(tab.root, tab.path) }
    else if (command.id === 'recovery') showCodingHistory()
    else if (command.id === 'history' && activeTab?.kind === 'codingFile') showCodingHistory(activeTab.root, activeTab.path)
    else if (command.id === 'terminal') store.openTerminalTab(`coding-${Date.now()}`, 'Terminal')
    else if (command.id === 'settings') store.openSettingsTab()
    else window.dispatchEvent(new CustomEvent('koma-coding-command', { detail: { id: command.id, tabId: activeTab?.id } }))
  }
  return (
    <div className="absolute inset-0 z-50 bg-black/15" onMouseDown={close}>
      <div role="dialog" aria-modal="true" aria-label={mode === 'files' ? 'Go to File' : 'Coding Commands'} className="mx-auto mt-12 w-[min(560px,90vw)] overflow-hidden rounded-md border border-koma-border bg-koma-panel shadow-xl" onMouseDown={e => e.stopPropagation()} onKeyDown={e => {
        if (e.key === 'Escape') { e.preventDefault(); close() }
        else if (e.key === 'ArrowDown') { e.preventDefault(); setSelected(i => Math.max(0, Math.min(i + 1, count - 1))) }
        else if (e.key === 'ArrowUp') { e.preventDefault(); setSelected(i => Math.max(0, i - 1)) }
        else if (e.key === 'Enter') { e.preventDefault(); pick(selected) }
        else if (e.key === 'Tab') {
          const items = Array.from(e.currentTarget.querySelectorAll<HTMLElement>('input,select,button:not([disabled])'))
          const index = items.indexOf(document.activeElement as HTMLElement)
          e.preventDefault(); items[(index + (e.shiftKey ? -1 : 1) + items.length) % items.length]?.focus()
        }
      }}>
        <div className="flex items-center gap-2 border-b border-koma-border px-3 py-2">
          {busy && mode === 'files' ? <BrailleSpinner size={14} /> : <Search size={14} className="text-koma-dim" />}
          <input ref={input} value={query} onChange={e => { setQuery(e.target.value); setSelected(0) }} placeholder={mode === 'files' ? 'Search workspace files…' : 'Search coding commands…'} aria-label="Search" className="min-w-0 flex-1 bg-transparent text-[12px] text-koma-fg outline-none" />
          <button onClick={close} aria-label="Close" className="rounded p-1 text-koma-dim hover:bg-koma-hover"><X size={13} /></button>
        </div>
        {mode === 'files' && <div className="flex items-center gap-2 border-b border-koma-border px-3 py-1.5 text-[11px] text-koma-dim">
          <span>Workspace</span>
          <select aria-label="Workspace" value={scope || activeRoot || ''} onChange={e => setScope(e.target.value)} className="min-w-0 flex-1 bg-koma-panel text-koma-fg outline-none">
            {(roots ?? []).map(root => <option key={root} value={root}>{root}</option>)}
            {(roots?.length ?? 0) > 1 && <option value="*">All workspaces</option>}
          </select>
        </div>}
        <div className="max-h-[55vh] overflow-y-auto py-1" role="listbox" aria-label="Results">
          {error && <div className="px-3 py-3 text-[12px] text-koma-error">{error}</div>}
          {!error && count === 0 && <div className="px-3 py-3 text-[12px] text-koma-dim">{busy ? 'Searching…' : 'No results'}</div>}
          {(mode === 'files' ? hits : choices).map((entry, i) => <button type="button" role="option" aria-selected={i === selected} key={'path' in entry ? JSON.stringify(entry) : entry.id} onMouseEnter={() => setSelected(i)} onClick={() => pick(i)} ref={node => { if (i === selected) node?.scrollIntoView({ block: 'nearest' }) }} className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] text-koma-fg ${i === selected ? 'bg-koma-hover' : 'hover:bg-koma-hover'}`}>
            {'path' in entry ? <File size={13} className="flex-none text-koma-dim" /> : <Terminal size={13} className="flex-none text-koma-dim" />}
            <span className="min-w-0 flex-1 truncate">{'path' in entry ? entry.path : entry.title}</span>
            <span className="max-w-[40%] truncate text-[10px] text-koma-dim">{'path' in entry ? (scope === '*' ? entry.root : '') : entry.shortcut}</span>
          </button>)}
        </div>
        {truncated && mode === 'files' && <div className="border-t border-koma-border px-3 py-1.5 text-[11px] text-koma-dim">Showing a limited result set. Narrow your search.</div>}
      </div>
    </div>
  )
}
