import { motion } from 'framer-motion'
import { Check, ShieldAlert, X } from 'lucide-react'
import { useKoma } from '../store/koma'

// Pretty-print the raw stringified-JSON tool args for the approval card. Falls
// back to the raw string on any parse miss (a non-JSON arg blob).
function formatArgs(args: string): string {
  try {
    const parsed = JSON.parse(args)
    return JSON.stringify(parsed, null, 2)
  } catch {
    return args
  }
}

// Host-authoritative tool-approval modal — the GUI equivalent of the TUI's
// paused-call y/a/n footer (approval.rs). Mounts whenever the session is parked
// on a RISKY / classifier-flagged tool call (`awaitingApproval` set + a
// `pendingCall` whose name is NOT `plan_ready`; the plan_ready pause is answered
// inline in the chat by the plan controls). Renders the tool name + args + the
// classifier's reason ("why"), and round-trips Approve/Deny via
// GuiReq::ApproveTool. Always-mounted store-driven overlay (SwitchingOverlay
// pattern), gated on the projected approval state — never user-initiated.
export function ApprovalOverlay() {
  const awaiting = useKoma((s) => s.session.awaitingApproval)
  const pending = useKoma((s) => s.session.pendingCall)
  const reason = useKoma((s) => s.session.approvalReason)
  const req = useKoma((s) => s.req)

  // Only a non-plan pause is a modal approval; plan_ready is handled inline.
  if (!awaiting || !pending || pending.name === 'plan_ready') return null

  const answer = (approve: boolean) => req({ r: 'ApproveTool', approve })
  const signature = pending.signature || pending.name
  const args = formatArgs(pending.args)

  return (
    <div className="absolute inset-0 z-[60] flex items-center justify-center bg-koma-bg/80 backdrop-blur-sm">
      <motion.div
        initial={{ opacity: 0, scale: 0.97, y: 6 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ duration: 0.16, ease: 'easeOut' }}
        className="mx-4 flex max-h-[70vh] w-[520px] max-w-full flex-col overflow-hidden rounded-lg border border-koma-warn/50 bg-koma-panel shadow-2xl"
      >
        <div className="flex items-center gap-2 border-b border-koma-border px-4 py-2.5 text-koma-warn">
          <ShieldAlert size={16} className="flex-none" />
          <span className="text-[13px] font-semibold">Approval required</span>
        </div>

        <div className="min-h-0 flex-1 overflow-auto px-4 py-3">
          <div className="text-[11px] uppercase tracking-wider text-koma-fg opacity-45">Tool</div>
          <div className="mt-0.5 break-words font-mono text-[13px] text-koma-fg">{signature}</div>

          {reason && reason.trim() !== '' && (
            <>
              <div className="mt-3 text-[11px] uppercase tracking-wider text-koma-fg opacity-45">Reason</div>
              <div className="mt-0.5 whitespace-pre-wrap break-words text-[12.5px] text-koma-warn">{reason}</div>
            </>
          )}

          <div className="mt-3 text-[11px] uppercase tracking-wider text-koma-fg opacity-45">Arguments</div>
          <pre className="mt-1 max-h-[28vh] overflow-auto rounded border border-koma-border bg-koma-bg px-2 py-1.5 font-mono text-[11.5px] leading-snug text-koma-dim">
            {args}
          </pre>
        </div>

        <div className="flex items-center justify-end gap-2 border-t border-koma-border px-4 py-2.5">
          <button
            onClick={() => answer(false)}
            className="flex items-center gap-1.5 rounded-md border border-koma-border bg-koma-panel px-3 py-1.5 text-[12px] text-koma-fg opacity-80 transition-colors hover:bg-koma-hover hover:opacity-100"
          >
            <X size={13} className="flex-none" />
            Deny
          </button>
          <button
            onClick={() => answer(true)}
            className="flex items-center gap-1.5 rounded-md border border-koma-accent bg-koma-accent/15 px-3 py-1.5 text-[12px] text-koma-accent transition-colors hover:bg-koma-accent/25"
          >
            <Check size={13} className="flex-none" />
            Approve
          </button>
        </div>
      </motion.div>
    </div>
  )
}
