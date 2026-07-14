import { useMemo } from 'react'
import { motion } from 'framer-motion'
import { Check, ShieldAlert, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import { fallbackSignature } from '../lib/toolSignature'

// Keys we surface as the dedicated "path" line, in priority order.
const PATH_KEYS = ['path', 'file_path', 'file', 'filename', 'dir', 'directory']
// Keys we surface as the main "content" block, in priority order.
const CONTENT_KEYS = ['content', 'new_string', 'command', 'text', 'body', 'code', 'query', 'old_string']

type HumanArgs = {
  path?: string
  contentLabel?: string
  content?: string
  rest: [string, string][]
}

// Split the stringified-JSON tool args into a human layout: a path line, a main
// content block (rendered with REAL newlines, not escaped \n), and any leftover
// scalar fields. Returns null when args aren't a JSON object so the caller can
// fall back to the raw string.
function humanizeArgs(args: string): HumanArgs | null {
  let parsed: unknown
  try {
    parsed = JSON.parse(args)
  } catch {
    return null
  }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return null
  const obj = parsed as Record<string, unknown>

  const consumed = new Set<string>()

  let path: string | undefined
  for (const k of PATH_KEYS) {
    if (typeof obj[k] === 'string') {
      path = obj[k] as string
      consumed.add(k)
      break
    }
  }

  let contentLabel: string | undefined
  let content: string | undefined
  for (const k of CONTENT_KEYS) {
    if (typeof obj[k] === 'string') {
      contentLabel = k
      content = obj[k] as string
      consumed.add(k)
      break
    }
  }

  const rest: [string, string][] = []
  for (const [k, v] of Object.entries(obj)) {
    if (consumed.has(k)) continue
    rest.push([k, typeof v === 'string' ? v : JSON.stringify(v)])
  }

  return { path, contentLabel, content, rest }
}

// Raw pretty-print fallback for non-object arg blobs.
function formatArgs(args: string): string {
  try {
    return JSON.stringify(JSON.parse(args), null, 2)
  } catch {
    return args
  }
}

// Host-authoritative tool-approval modal — the GUI equivalent of the TUI's
// paused-call y/a/n footer (approval.rs). Mounts whenever the session is parked
// on a RISKY / classifier-flagged tool call (`awaitingApproval` set + a
// `pendingCall` whose name is NOT `plan_ready`; the plan_ready pause is answered
// inline in the chat by the plan controls). Renders the tool name + reason inline,
// then the target path and the tool content, and round-trips Approve/Deny via
// GuiReq::ApproveTool. Always-mounted store-driven overlay (SwitchingOverlay
// pattern), gated on the projected approval state — never user-initiated.
export function ApprovalOverlay() {
  const awaiting = useKoma((s) => s.session.awaitingApproval)
  const pending = useKoma((s) => s.session.pendingCall)
  const reason = useKoma((s) => s.session.approvalReason)
  const req = useKoma((s) => s.req)
  const theme = useKoma((s) => s.config.theme)
  const palettes = useKoma((s) => s.config.palettes)

  // Severity lives ONLY in the title icon's tint, derived from the active
  // palette's `warn` role colour (same derivation as ToastContainer — index 8
  // of the fixed 11-role `colors` array). Falls back to themed fg when the
  // active palette isn't advertised yet.
  const warnColor = useMemo(() => {
    const active = palettes.find((p) => p.name === theme)
    return active?.colors?.[8] || 'var(--koma-fg)'
  }, [palettes, theme])

  // Only a non-plan pause is a modal approval; plan_ready is handled inline.
  if (!awaiting || !pending || pending.name === 'plan_ready') return null

  const answer = (approve: boolean) => req({ r: 'ApproveTool', approve })
  const signature = pending.signature || fallbackSignature(pending.name, pending.args)
  const human = humanizeArgs(pending.args)
  const hasReason = !!(reason && reason.trim() !== '')

  return (
    <div className="px-2 pb-1">
      <motion.div
        initial={{ opacity: 0, scale: 0.97, y: 6 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ duration: 0.16, ease: 'easeOut' }}
        className="flex max-h-[40vh] w-full flex-col overflow-hidden rounded-xl border border-koma-border bg-koma-panel shadow-lg"
      >
        <div className="flex items-center gap-2 border-b border-koma-border px-4 py-2.5 text-koma-fg">
          <ShieldAlert size={16} className="flex-none" style={{ color: warnColor }} />
          <span className="text-[13px] font-semibold">Approval required</span>
        </div>

        <div className="min-h-0 flex-1 overflow-auto px-4 py-3">
          {/* Tool + reason, inline. */}
          <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
            <span className="break-all font-mono text-[13px] text-koma-fg">{signature}</span>
            {hasReason && (
              <>
                <span className="text-koma-dim opacity-40">·</span>
                <span className="break-words text-[12px] text-koma-dim">{reason}</span>
              </>
            )}
          </div>

          {human ? (
            <>
              {human.path !== undefined && (
                <div className="mt-3">
                  <div className="text-[10px] uppercase tracking-wider text-koma-fg opacity-45">Path</div>
                  <div className="mt-0.5 break-all font-mono text-[12.5px] text-koma-accent">{human.path}</div>
                </div>
              )}

              {human.content !== undefined && (
                <div className="mt-3">
                  <div className="text-[10px] uppercase tracking-wider text-koma-fg opacity-45">
                    {human.contentLabel === 'content' ? 'Content' : human.contentLabel}
                  </div>
                  <pre className="mt-1 max-h-[22vh] overflow-auto whitespace-pre-wrap break-words rounded border border-koma-border bg-koma-bg px-2.5 py-2 font-mono text-[11.5px] leading-snug text-koma-dim">
                    {human.content}
                  </pre>
                </div>
              )}

              {human.rest.length > 0 && (
                <div className="mt-3 space-y-0.5">
                  {human.rest.map(([k, v]) => (
                    <div key={k} className="flex flex-wrap items-baseline gap-x-1.5">
                      <span className="font-mono text-[11px] text-koma-fg opacity-45">{k}:</span>
                      <span className="break-all font-mono text-[11.5px] text-koma-dim">{v}</span>
                    </div>
                  ))}
                </div>
              )}
            </>
          ) : (
            <>
              <div className="mt-3 text-[10px] uppercase tracking-wider text-koma-fg opacity-45">Arguments</div>
              <pre className="mt-1 overflow-auto whitespace-pre-wrap break-words rounded border border-koma-border bg-koma-bg px-2.5 py-2 font-mono text-[11.5px] leading-snug text-koma-dim">
                {formatArgs(pending.args)}
              </pre>
            </>
          )}
        </div>

        <div className="flex items-center justify-start gap-2 border-t border-koma-border px-4 py-2.5">
          <button
            onClick={() => answer(true)}
            className="flex items-center gap-1.5 rounded-md border border-koma-accent bg-koma-accent/15 px-3 py-1.5 text-[12px] text-koma-accent transition-colors hover:bg-koma-accent/25"
          >
            <Check size={13} className="flex-none" />
            Approve
          </button>
          <button
            onClick={() => answer(false)}
            className="flex items-center gap-1.5 rounded-md border border-koma-border bg-koma-panel px-3 py-1.5 text-[12px] text-koma-fg opacity-80 transition-colors hover:bg-koma-hover hover:opacity-100"
          >
            <X size={13} className="flex-none" />
            Deny
          </button>
        </div>
      </motion.div>
    </div>
  )
}
