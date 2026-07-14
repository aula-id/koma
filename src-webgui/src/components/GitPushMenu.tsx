import { useEffect, useRef, useState } from 'react'
import { ArrowUp, ChevronDown, ShieldAlert } from 'lucide-react'
import { useKoma, type GitPushMode } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'

type Props = { compact?: boolean; badge?: number }

const LABEL: Record<GitPushMode, string> = {
  automatic: 'Push',
  plain: 'Push',
  'set-upstream': 'Push & set upstream',
  'force-with-lease': 'Force push with lease',
}

/** Shared authoritative push control used by both graph and Source Control.
 * Automatic is the primary action; explicit force is shown only when the host
 * status planner has proved a GUI rebase and requires a second confirmation.
 */
export function GitPushMenu({ compact = false, badge }: Props) {
  const planned = useKoma((s) => s.git.pushMode)
  const busy = useKoma((s) => s.remoteBusy)
  const push = useKoma((s) => s.gitPush)
  const [open, setOpen] = useState(false)
  const [confirmForce, setConfirmForce] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  const disabled = !!busy || !planned

  useEffect(() => {
    if (!open) return
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false)
    }
    window.addEventListener('mousedown', close)
    return () => window.removeEventListener('mousedown', close)
  }, [open])

  const run = (mode: GitPushMode) => {
    if (mode === 'force-with-lease') {
      setConfirmForce(true)
      return
    }
    setOpen(false)
    push(mode)
  }

  return (
    <div ref={ref} className="relative flex flex-none">
      <button
        type="button"
        disabled={disabled}
        onClick={() => {
          if (planned === 'force-with-lease') {
            setOpen(true)
            setConfirmForce(true)
          } else {
            push('automatic')
          }
        }}
        title={planned ? LABEL[planned] : 'Push unavailable: refresh or configure a remote'}
        className={`flex items-center gap-1 text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100 disabled:cursor-not-allowed disabled:opacity-30 ${compact ? 'rounded-l px-1.5 py-0.5' : 'rounded-l px-1.5 py-1'}`}
      >
        {busy === 'push' ? <BrailleSpinner size={13} /> : <ArrowUp size={compact ? 12 : 13} />}
        {!compact && <span className="text-[11px]">{planned === 'set-upstream' ? 'Publish' : 'Push'}</span>}
        {!!badge && badge > 0 && <span className="font-mono text-[10px]">{badge}</span>}
      </button>
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen((v) => {
          if (!v) setConfirmForce(false)
          return !v
        })}
        aria-label="Push options"
        className="rounded-r border-l border-koma-border px-0.5 text-koma-fg opacity-60 hover:bg-koma-hover disabled:opacity-30"
      >
        <ChevronDown size={11} />
      </button>
      {open && (
        <div className="absolute left-0 top-full z-[95] mt-1 min-w-[190px] rounded border border-koma-border bg-koma-panel p-1 shadow-sm">
          {confirmForce ? (
            <div className="flex flex-col gap-2 p-2 text-[11px]">
              <span className="flex gap-1.5 text-koma-error"><ShieldAlert size={13} />Rewrite the remote branch using an exact lease?</span>
              <div className="flex gap-1">
                <button type="button" onClick={() => { setOpen(false); setConfirmForce(false); push('force-with-lease') }} className="rounded bg-koma-error px-2 py-1 font-semibold text-koma-bg">Force push</button>
                <button type="button" onClick={() => setConfirmForce(false)} className="rounded px-2 py-1 hover:bg-koma-hover">Cancel</button>
              </div>
            </div>
          ) : (
            <>
              <button type="button" onClick={() => run('plain')} className="block w-full rounded px-2 py-1 text-left text-[11px] hover:bg-koma-hover">Push</button>
              <button type="button" onClick={() => run('set-upstream')} className="block w-full rounded px-2 py-1 text-left text-[11px] hover:bg-koma-hover">Push &amp; set upstream</button>
              {planned === 'force-with-lease' && <button type="button" onClick={() => run('force-with-lease')} className="block w-full rounded px-2 py-1 text-left text-[11px] text-koma-error hover:bg-koma-hover">Force push with lease…</button>}
            </>
          )}
        </div>
      )}
    </div>
  )
}
