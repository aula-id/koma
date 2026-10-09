import { useEffect, useReducer } from 'react'
import { useKoma } from '../store/koma'
import { SEARCH_PROVIDERS, activation, canEnable, initialSearchSettings, searchSettingsReducer, type SearchProvider } from '../types/web-search'

let nextRequest = 0
export function WebSearchSettings() {
  const req = useKoma(s => s.req)
  const openExternal = useKoma(s => s.openExternal)
  const reply = useKoma(s => s.webSearchValues)
  const [state, dispatch] = useReducer(searchSettingsReducer, initialSearchSettings)
  useEffect(() => {
    const seq = ++nextRequest
    dispatch({ type: 'load', seq })
    req({ r: 'GetWebSearch', req_seq: seq })
  }, [req])
  useEffect(() => { if (reply) dispatch({ type: 'reply', reply }) }, [reply])
  const { status, drafts, pending, error } = state
  if (!status) return <p className="text-xs opacity-60">Loading web search settings…</p>
  const toggle = (provider: SearchProvider) => {
    if (pending) return
    const update = activation(status, provider, drafts[provider] ?? '')
    if (!update) return
    const seq = ++nextRequest
    dispatch({ type: 'begin', seq, provider })
    req({ r: 'SetWebSearch', req_seq: seq, ...update })
  }
  return <div className="flex flex-col gap-3">
    {SEARCH_PROVIDERS.map(({ id, label, website }) => {
      const active = status.provider === id
      const disabled = pending !== null || (active && id === 'built_in') || !canEnable(status, id, drafts[id] ?? '')
      return <div key={id} className="rounded border border-koma-border bg-koma-panel2 px-3 py-2.5">
        <div className="flex flex-col items-start gap-2">
          <a href={website} onClick={e => { e.preventDefault(); openExternal(website) }} className="text-[13px] hover:underline">{label}</a>
          <label className="flex flex-col items-start gap-1.5 text-xs">
            Enable
            <button type="button" role="switch" aria-label={`Enable ${label}`} aria-checked={active} disabled={disabled} onClick={() => toggle(id)}
              className={`relative h-4 w-7 rounded-full disabled:opacity-40 ${active ? 'bg-emerald-500/70' : 'bg-koma-grip'}`}>
              <span className={`absolute top-0.5 h-3 w-3 rounded-full bg-white ${active ? 'left-[14px]' : 'left-0.5'}`} />
            </button>
          </label>
        </div>
        {id !== 'built_in' && <div className="mt-2">
          <label htmlFor={`settings-search-key-${id}`} className="mb-1 block text-xs">API key</label>
          <input id={`settings-search-key-${id}`} type="password" aria-label={`${label} API key`} autoComplete="new-password" value={drafts[id] ?? ''}
            disabled={active || pending !== null} placeholder={status.saved_keys.includes(id) ? 'Key saved' : 'API key'}
            onChange={e => dispatch({ type: 'edit', provider: id, key: e.target.value })}
            className="w-full rounded border border-koma-border bg-koma-bg px-2 py-1 text-xs disabled:opacity-50" />
          <p className="mt-1 text-[11px] opacity-50">To replace a key, turn Enable off, edit the key, then turn Enable on.</p>
        </div>}
      </div>
    })}
    {pending !== null && <p role="status" className="text-xs opacity-60">Saving web search settings…</p>}
    {error && <p role="alert" className="text-xs text-red-400">{error}</p>}
  </div>
}
