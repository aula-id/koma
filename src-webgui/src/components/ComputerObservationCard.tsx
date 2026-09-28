import { useState } from 'react'
import { Check, ChevronDown, LoaderCircle, Monitor, ScanEye, AlertCircle } from 'lucide-react'
import type { ComputerObservationView } from '../types/computer'
import type { ToolCallView } from '../store/koma'
import { computerImageUrl } from './ComputerPreview'

export function ComputerObservationCard({ observation: o }: { observation: ComputerObservationView }) {
  const [expanded, setExpanded] = useState(false)
  const [missing, setMissing] = useState(false)
  return <article className="overflow-hidden rounded-lg border border-koma-border bg-koma-panel/40 text-koma-fg">
    <button type="button" aria-expanded={expanded} onClick={() => setExpanded(v => !v)} className="flex w-full items-center gap-3 p-3 text-left hover:bg-koma-hover">
      <div className="flex h-12 w-20 shrink-0 items-center justify-center overflow-hidden rounded border border-koma-border bg-koma-bg">
        {missing ? <Monitor size={20} className="text-koma-dim" /> : <img src={computerImageUrl(o.image_path)} alt="" loading="lazy" onError={() => setMissing(true)} className="h-full w-full object-contain" />}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5 text-[10px] text-koma-dim"><ScanEye size={12} /><span>Model observation</span><span>· {new Date(o.captured_ms).toLocaleTimeString()}</span></div>
        <p className="mt-1 truncate text-[12.5px]">{o.title || o.application}</p>
        <p className="mt-0.5 truncate text-[10px] text-koma-dim">{o.application} · {o.width} × {o.height}</p>
      </div>
      <ChevronDown size={14} className={`shrink-0 text-koma-dim transition-transform ${expanded ? 'rotate-180' : ''}`} />
    </button>
    {expanded && <div className="border-t border-koma-border">
      {missing ? <p className="p-4 text-xs text-koma-dim">This saved frame is no longer available.</p> : <img src={computerImageUrl(o.image_path)} alt={`Exact frame sent to the model: ${o.title || o.application}`} loading="lazy" onError={() => setMissing(true)} className="max-h-[65vh] w-full object-contain" />}
      <div className="space-y-1.5 border-t border-koma-border px-3 py-2 text-[11px] leading-relaxed text-koma-dim">
        <p>Exact frame sent to the model. The live preview may show a newer frame.</p>
        <details><summary className="cursor-pointer">Observation details</summary><p className="mt-2 break-words">Accessibility: {o.accessibility_status}</p><p className="mt-1 break-words">Text recognition: {o.ocr_status}</p><p className="mt-1 break-all">Observation: {o.id}</p></details>
      </div>
    </div>}
  </article>
}

const labels: Record<string, string> = { computer_windows: 'Find sources', computer_select_window: 'Share source', computer_observe: 'Observe computer', computer_act: 'Use computer' }
export function ComputerToolCall({ call }: { call: ToolCallView }) {
  let result: { completed?: number; uncertain?: boolean; input_busy?: boolean; error?: string | null; windows?: unknown[]; controller_enabled?: boolean; requires_observation?: boolean; requires_screen?: boolean; recovery?: { user_action?: string } } | null = null
  try { const value: unknown = JSON.parse(call.output ?? 'null'); if (value && typeof value === 'object') result = value } catch { /* Plain error output remains visible. */ }
  const error = typeof result?.error === 'string' ? result.error : !result && /^(error:|cancelled|native operation failed)/i.test(call.output ?? '') ? call.output : null
  const uncertain = result?.uncertain === true
  const done = call.status === 'done'
  const recovery = typeof result?.recovery?.user_action === 'string' ? result.recovery.user_action : null
  const summary = !done ? 'Working…'
    : result?.controller_enabled === false ? 'Stopped'
    : result?.input_busy && !uncertain ? 'Input busy'
    : result?.requires_screen ? 'Select a screen'
    : result?.requires_observation ? 'Observe again'
    : error ? 'Failed'
    : uncertain ? 'Needs attention'
    : call.name === 'computer_act' && typeof result?.completed === 'number' ? `${result.completed} input${result.completed === 1 ? '' : 's'} completed`
    : call.name === 'computer_windows' && Array.isArray(result?.windows) ? `${result.windows.length} sources available`
    : 'Completed'
  // Keep the text on the active palette's readable foreground. Semantic colors
  // are cues on the icons, not low-contrast tints across the entire message.
  return <div className="rounded-md border border-koma-border bg-koma-panel/30 text-xs text-koma-fg"><details>
    <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-2">
      {!done ? <LoaderCircle size={13} className="shrink-0 animate-spin text-koma-accent" /> : error || uncertain ? <AlertCircle size={13} className={`shrink-0 ${error && !result?.requires_screen && !result?.input_busy ? 'text-koma-error' : 'text-koma-warn'}`} /> : <Check size={13} className="shrink-0 text-koma-accent" />}
      <span className="shrink-0">{labels[call.name] || call.name}</span><span className={`min-w-0 flex-1 ${error || uncertain ? 'text-koma-fg' : 'truncate text-koma-dim'}`}>{summary}</span><ChevronDown size={12} className="shrink-0 text-koma-dim" />
    </summary>
    <div className="max-h-72 space-y-3 overflow-auto border-t border-koma-border p-3 text-xs leading-relaxed text-koma-fg">
      {error && <p>{error}</p>}
      {uncertain && <p>Some input may have completed. Inspect a fresh observation before deciding what to do next.</p>}
      {result?.requires_screen && <p>This share cannot receive input. On Wayland the model selects a screen. On macOS, Windows, and X11 it shares the application window and clicks inside that screenshot.</p>}
      {result?.input_busy && !uncertain && <p>Release held keys or mouse buttons. Sharing is still active; the agent can observe and continue.</p>}
      {result?.requires_observation && !result?.input_busy && <p>The desktop changed. Sharing is still active; a fresh observation is required.</p>}
      {recovery && <p><strong>Computer control stopped. </strong>{recovery}</p>}
      <details className="text-[11px] text-koma-dim"><summary className="cursor-pointer">Technical details</summary>
        <div className="mt-2 space-y-2"><p>Request</p><pre className="whitespace-pre-wrap break-all">{call.args}</pre>{call.output && <><p>Result</p><pre className="whitespace-pre-wrap break-all">{call.output}</pre></>}</div>
      </details>
    </div>
  </details></div>
}
