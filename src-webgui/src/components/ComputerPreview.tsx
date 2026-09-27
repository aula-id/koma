import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Check, ChevronDown, Monitor, RefreshCw } from 'lucide-react'
import { useComputerLivePreview } from '../lib/computerLivePreview'
import type { ComputerStatus } from '../types/computer'

export function computerImageUrl(path: string) {
  const origin = ['http:', 'https:'].includes(location.protocol) ? location.origin : 'koma://localhost'
  return `${origin}/image/${encodeURIComponent(path)}`
}

/** Live selected-window view, with a clearly labelled saved-observation fallback. */
export function ComputerPreview({ status, control, chrome, onImageSize }: {
  status: ComputerStatus | null
  control: (action: 'windows' | 'select', window?: string) => void
  chrome?: ReactNode
  onImageSize?: (width: number, height: number) => void
}) {
  const [menu, setMenu] = useState(false)
  const [failedImage, setFailedImage] = useState<string | null>(null)
  const root = useRef<HTMLDivElement>(null)
  const trigger = useRef<HTMLButtonElement>(null)
  const observation = status?.observation
  const live = useComputerLivePreview(status)
  const image = live?.image ?? (observation ? computerImageUrl(observation.image_path) : null)
  const unavailable = !status?.enabled || status.paused || status.busy
  useEffect(() => { setMenu(false) }, [status?.session, status?.generation])
  useEffect(() => {
    if (!menu) return
    const outside = (e: PointerEvent) => { if (!root.current?.contains(e.target as Node)) setMenu(false) }
    document.addEventListener('pointerdown', outside)
    return () => document.removeEventListener('pointerdown', outside)
  }, [menu])

  return <div ref={root} className="group/preview relative h-full min-h-0 w-full overflow-hidden bg-koma-bg text-koma-fg"
    onKeyDown={e => { if (e.key === 'Escape' && menu) { e.stopPropagation(); setMenu(false); trigger.current?.focus() } }}>
    {image && failedImage !== image
      ? <img src={image} draggable={false}
          onLoad={e => onImageSize?.(e.currentTarget.naturalWidth, e.currentTarget.naturalHeight)}
          onError={() => setFailedImage(image)}
          alt={`${live?.image ? 'Live preview' : 'Last model observation'}: ${observation?.window.title || observation?.window.application}`}
          className="absolute inset-0 block h-full w-full" />
      : <div className="flex h-full flex-col items-center justify-center gap-3 px-8 text-center text-koma-dim">
          <Monitor size={30} strokeWidth={1.25} />
          <p className="text-sm">{observation ? 'Saved frame unavailable' : 'Choose a window to share'}</p>
          <p className="max-w-64 text-xs leading-relaxed">{observation ? 'The image could not be loaded from this session.' : 'Watch the selected window live. Model observations appear separately in chat.'}</p>
        </div>}

    <div className={`pointer-events-none absolute inset-x-0 top-0 flex max-h-full flex-col p-2 transition-opacity duration-150 group-hover/preview:opacity-100 group-focus-within/preview:opacity-100 [@media(hover:none)]:opacity-100 ${menu || !observation ? 'opacity-100' : 'opacity-0'}`}>
      <div className="pointer-events-auto flex shrink-0 items-center gap-1 rounded-lg border border-koma-border bg-koma-panel/95 p-1 shadow-lg backdrop-blur-md">
        <button ref={trigger} type="button" aria-expanded={menu} aria-controls="computer-window-picker" disabled={!status?.enabled}
          onClick={() => { setMenu(v => !v); if (!menu && !unavailable && status?.capabilities.windows) control('windows') }}
          className="flex min-w-0 flex-1 items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs hover:bg-koma-hover focus-visible:outline focus-visible:outline-koma-accent disabled:opacity-50">
          <Monitor size={14} className="shrink-0 text-koma-accent" />
          <span className="min-w-0 flex-1 truncate">{observation?.window.title || observation?.window.application || 'Select window'}</span>
          <ChevronDown size={13} className={`shrink-0 transition-transform ${menu ? 'rotate-180' : ''}`} />
        </button>
        {chrome}
      </div>
      {menu && <div id="computer-window-picker" aria-label="Windows to share" className="pointer-events-auto mt-1 min-h-0 max-h-60 overflow-y-auto rounded-lg border border-koma-border bg-koma-panel p-1 shadow-xl">
        <div className="flex items-center justify-between px-2 py-2 text-[10px] uppercase tracking-wider text-koma-dim">
          <span>Share a window</span>
          {status?.capabilities.windows && <button type="button" title="Refresh windows" aria-label="Refresh windows" disabled={unavailable} onClick={() => control('windows')} className="rounded p-1 hover:bg-koma-hover disabled:opacity-40"><RefreshCw size={12} className={status.busy ? 'animate-spin' : ''} /></button>}
        </div>
        {status?.windows.filter(w => w.id !== 'portal:choose').map(w => <button key={w.id} type="button" disabled={unavailable} aria-pressed={w.id === observation?.window.id}
          onClick={() => { control('select', w.id); setMenu(false); trigger.current?.focus() }}
          className="flex w-full items-center gap-2 rounded-md px-2 py-2 text-left hover:bg-koma-hover focus-visible:outline focus-visible:outline-koma-accent disabled:opacity-40">
          <Monitor size={14} className="shrink-0 text-koma-dim" />
          <span className="min-w-0 flex-1"><span className="block truncate text-xs">{w.title || w.application}</span><span className="block truncate text-[10px] text-koma-dim">{w.application} · {Math.round(w.geometry.width)} × {Math.round(w.geometry.height)}</span></span>
          {w.id === observation?.window.id && <Check size={13} className="shrink-0 text-koma-accent" />}
        </button>)}
        {status?.capabilities.capture && !status.capabilities.windows && <button type="button" disabled={unavailable} className="w-full rounded-md px-2 py-2 text-left text-xs hover:bg-koma-hover disabled:opacity-40" onClick={() => { control('select', 'portal:choose'); setMenu(false) }}>Choose source in system dialog…</button>}
        {!status?.windows.length && status?.capabilities.windows && <p className="px-2 py-3 text-xs text-koma-dim">{status.busy ? 'Finding windows…' : 'No windows available. Refresh to try again.'}</p>}
      </div>}
    </div>
    <div className="pointer-events-none absolute inset-x-0 bottom-0 flex justify-center p-2 opacity-0 transition-opacity group-hover/preview:opacity-100 group-focus-within/preview:opacity-100 [@media(hover:none)]:opacity-100">
      <span title={live?.error ?? undefined} className="max-w-full truncate rounded-full border border-koma-border bg-koma-panel/95 px-3 py-1 text-[10px] text-koma-dim shadow-sm">
        {!status?.enabled ? 'Control stopped' : status.paused ? 'Paused' : status.busy ? 'Updating…' : live?.image ? 'Live' : live?.error ? 'Live unavailable · last model frame' : 'Last model frame'}
        {observation && ` · ${new Date(live?.image ? live.captured_ms : observation.captured_ms).toLocaleTimeString()}`}
      </span>
    </div>
  </div>
}
