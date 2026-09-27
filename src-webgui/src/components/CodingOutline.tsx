import { useEffect, useState } from 'react'
import { X } from 'lucide-react'
import { useKoma } from '../store/koma'
import { fileKey } from '../store/coding'
import { mintId, pathToUri, trackDocumentSymbol, type LspDocumentSymbol } from '../lib/lsp-bridge'
import { flushPendingLspDidChange } from '../lib/monaco-lsp'

export function CodingOutline({ root, path, onClose }: { root: string; path: string; onClose: () => void }) {
  const content = useKoma(s => s.coding.files[fileKey(root, path)]?.content)
  const hostId = useKoma(s => s.remoteState.hostId ?? 'local')
  const [symbols, setSymbols] = useState<LspDocumentSymbol[]>([])
  const [query, setQuery] = useState('')
  const [error, setError] = useState('')
  useEffect(() => {
    let stopped = false
    const timer = setTimeout(async () => {
      try {
        await flushPendingLspDidChange(root, path)
        if (stopped) return
        const requestId = mintId('outline')
        const response = trackDocumentSymbol(requestId)
        useKoma.getState().req({ r: 'LspDocumentSymbol', root, path, requestId })
        const result = await response
        if (!stopped) { setSymbols(result); setError('') }
      } catch (e) { if (!stopped) { setSymbols([]); setError(String(e instanceof Error ? e.message : e)) } }
    }, 450)
    return () => { stopped = true; clearTimeout(timer) }
  }, [root, path, hostId, content])
  return <aside aria-label="Document outline" className="flex w-52 max-w-[45%] flex-none flex-col border-l border-koma-border bg-koma-panel text-[11px]">
    <div className="flex h-7 items-center justify-between border-b border-koma-border px-2 text-koma-dim">OUTLINE<button aria-label="Close outline" onClick={onClose} className="rounded p-1 hover:bg-koma-hover"><X size={12} /></button></div>
    <input aria-label="Filter symbols" placeholder="Filter symbols…" value={query} onChange={e => setQuery(e.target.value)} className="m-2 min-w-0 rounded border border-koma-border bg-transparent px-2 py-1 text-koma-fg outline-none" />
    <div className="min-h-0 flex-1 overflow-auto">
      {error && <p className="px-2 text-koma-dim">{error}</p>}
      {!error && !symbols.length && <p className="px-2 text-koma-dim">No symbols</p>}
      {symbols.filter(s => s.name.toLowerCase().includes(query.toLowerCase())).map((symbol, index) => <button key={`${symbol.name}:${symbol.selectionRange.startLine}:${index}`} title={`${symbol.name} · line ${symbol.selectionRange.startLine + 1}`} onClick={() => useKoma.getState().openDiagnostic(pathToUri(root, path), symbol.selectionRange.startLine, symbol.selectionRange.startCharacter)} className="flex w-full items-center gap-2 px-2 py-1 text-left text-koma-fg hover:bg-koma-hover">
        <span className="min-w-0 flex-1 truncate">{symbol.name}</span><span className="text-koma-dim">{symbol.selectionRange.startLine + 1}</span>
      </button>)}
    </div>
  </aside>
}
