import { useEffect, useRef, useState } from 'react'
import { Check, Copy } from 'lucide-react'
import { useKoma, type OAuthProviderEntry } from '../../../store/koma'
import { BrailleSpinner } from '../../BrailleSpinner'

// Copy-to-clipboard button — no existing reusable affordance in this codebase
// to lift (the only prior "copy" surface is the markdown code-block copy
// button baked into the streamdown library, not a component of ours), so this
// is a small local one: icon + label, flips to a themed accent check for a
// beat after a successful copy.
function CopyButton({ text, label = 'Copy' }: { text: string; label?: string }) {
  const [copied, setCopied] = useState(false)
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    return () => {
      if (timeoutRef.current) clearTimeout(timeoutRef.current)
    }
  }, [])

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      if (timeoutRef.current) clearTimeout(timeoutRef.current)
      timeoutRef.current = setTimeout(() => setCopied(false), 1500)
    } catch {
      /* clipboard unavailable — no-op, the value is still shown/selectable on screen */
    }
  }
  return (
    <button
      type="button"
      onClick={copy}
      className="flex flex-none items-center gap-1 rounded border border-koma-border px-2 py-1 text-[11px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
    >
      {copied ? <Check size={12} className="flex-none text-koma-accent" /> : <Copy size={12} className="flex-none" />}
      {copied ? 'Copied' : label}
    </button>
  )
}

function CancelFooter({ onCancel }: { onCancel: () => void }) {
  return (
    <div className="flex flex-none items-center justify-end border-t border-koma-border px-3 py-2">
      <button
        onClick={onCancel}
        className="rounded px-2.5 py-1 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
      >
        Cancel
      </button>
    </div>
  )
}

// Provider picker — DATA-DRIVEN from the store's `oauth.providers` (never
// hardcode the provider list; it's designed to grow). `kind` drives the
// trailing hint text.
function Picker({
  providers,
  onPick,
  onCancel,
}: {
  providers: OAuthProviderEntry[]
  onPick: (id: string) => void
  onCancel: () => void
}) {
  return (
    <>
      <div className="flex-1 overflow-auto p-3">
        <div className="mb-2 text-[11px] text-koma-fg opacity-50">Choose a provider to connect</div>
        {providers.length === 0 ? (
          <div className="px-1 py-2 text-[12px] text-koma-fg opacity-35">No OAuth providers available</div>
        ) : (
          <div className="flex flex-col gap-1.5">
            {providers.map((p) => (
              <button
                key={p.id}
                onClick={() => onPick(p.id)}
                className="flex items-center justify-between rounded border border-koma-border px-3 py-2 text-[13px] text-koma-fg transition-colors hover:bg-koma-hover"
              >
                <span className="text-left">{p.label}</span>
                <span className="text-[10px] uppercase tracking-wide text-koma-fg opacity-40">
                  {p.kind === 'device'
                    ? 'device code'
                    : p.kind === 'paste'
                      ? 'paste token'
                      : p.kind === 'reuse'
                        ? 'reuse login'
                        : p.kind === 'callback'
                          ? 'browser callback'
                          : 'browser'}
                </span>
              </button>
            ))}
          </div>
        )}
      </div>
      <CancelFooter onCancel={onCancel} />
    </>
  )
}

function FailedView({
  error,
  onTryAgain,
  onCancel,
}: {
  error: string | null
  onTryAgain: () => void
  onCancel: () => void
}) {
  return (
    <>
      <div className="flex-1 overflow-auto p-3">
        <p className="text-[12px] text-koma-fg opacity-80">{error || 'Something went wrong.'}</p>
      </div>
      <div className="flex flex-none items-center justify-between border-t border-koma-border px-3 py-2">
        <button
          onClick={onCancel}
          className="rounded px-2.5 py-1 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
        >
          Cancel
        </button>
        <button
          onClick={onTryAgain}
          className="rounded border border-koma-border px-2.5 py-1 text-[12px] text-koma-fg transition-colors hover:bg-koma-hover"
        >
          Try again
        </button>
      </div>
    </>
  )
}

type Props = {
  // Leave the OAuth screen. IMPORTANT: this is expected to abort an in-flight
  // flow (CancelOAuth) before navigating away when one is running — see
  // ConnectorPanel's `leaveOAuth`, which both the DetailHeader back-arrow and
  // every Cancel button in this component route through, so leaving mid-flow
  // is coherent no matter which control triggers it (this component does NOT
  // fire CancelOAuth itself — that would double-fire alongside the header's
  // own abort check).
  onDone: () => void
}

// Multi-phase OAuth login flow. Lives entirely off the store's `oauth` slice
// (populated by the OAuthState push) — this component only ever fires
// requests and renders whatever phase the daemon currently reports; it holds
// no authoritative state of its own beyond the local "force the picker after
// a failure" flag and the paste-token draft.
export function OAuthConnect({ onDone }: Props) {
  const req = useKoma((s) => s.req)
  const oauth = useKoma((s) => s.oauth)
  const [token, setToken] = useState('')
  // "Try again" from a failed flow shows the picker locally even though the
  // store's `phase` is still 'failed' (only a fresh StartOAuth/CancelOAuth
  // changes it daemon-side). Reset the moment a REAL flow actually starts —
  // once phase moves off 'idle'/'failed' the authoritative phase takes over
  // regardless of this flag.
  const [pickerForced, setPickerForced] = useState(false)
  useEffect(() => {
    if (oauth.phase !== 'failed' && oauth.phase !== 'idle') setPickerForced(false)
  }, [oauth.phase])

  // 'success' is a terminal pulse, not a screen of its own: the SAME push
  // already carries the updated `conns`, so just settle the daemon back to a
  // clean 'idle' (covers a host build that doesn't auto-follow success with
  // its own idle push) and return to the list — matches "success: transition
  // back to idle list" from the locked design. `handledSuccess` makes this a
  // true one-shot: without it, GetOAuthState+onDone would refire on every
  // re-render while `oauth.phase` stays 'success' during the ~220ms exit
  // animation (onDone doesn't unmount this component synchronously).
  const handledSuccess = useRef(false)
  useEffect(() => {
    if (oauth.phase !== 'success' || handledSuccess.current) return
    handledSuccess.current = true
    req({ r: 'GetOAuthState' })
    onDone()
  }, [oauth.phase, req, onDone])

  const showPicker = oauth.phase === 'idle' || (oauth.phase === 'failed' && pickerForced)

  if (showPicker) {
    return (
      <Picker
        providers={oauth.providers}
        onPick={(id) => {
          setPickerForced(false)
          req({ r: 'StartOAuth', provider: id })
        }}
        onCancel={onDone}
      />
    )
  }

  if (oauth.phase === 'failed') {
    return <FailedView error={oauth.error} onTryAgain={() => setPickerForced(true)} onCancel={onDone} />
  }

  if (oauth.phase === 'starting') {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center">
        <BrailleSpinner size={20} className="text-koma-accent" />
        <span className="text-[12px] text-koma-fg opacity-60">starting…</span>
      </div>
    )
  }

  // codex-style PKCE: the daemon already opened a browser tab — this is the
  // fallback + live status while we wait for it to complete.
  if (oauth.phase === 'waiting_url') {
    return (
      <>
        <div className="flex-1 overflow-auto p-3">
          <p className="mb-3 text-[12px] text-koma-fg opacity-70">
            Your browser should have opened. If it didn't, open this URL:
          </p>
          <div className="flex items-center gap-2 rounded border border-koma-border bg-koma-bg px-2 py-1.5">
            <span className="min-w-0 flex-1 truncate text-[11px] text-koma-fg opacity-80">{oauth.url}</span>
            {oauth.url && <CopyButton text={oauth.url} />}
          </div>
          <div className="mt-3 flex items-center gap-1.5 text-[11px] text-koma-fg opacity-50">
            <BrailleSpinner size={12} />
            waiting for sign-in…
          </div>
        </div>
        <CancelFooter onCancel={onDone} />
      </>
    )
  }

  // kilocode-style device flow: a short code to enter at a verification page.
  if (oauth.phase === 'waiting_code') {
    return (
      <>
        <div className="flex-1 overflow-auto p-3">
          <p className="mb-3 text-[12px] text-koma-fg opacity-70">
            Enter this code at {oauth.verificationUrl || 'the verification page'}:
          </p>
          <div className="flex items-center justify-center rounded-lg border border-koma-border bg-koma-bg px-3 py-4">
            <span className="select-all font-mono text-[22px] font-semibold tracking-widest text-koma-fg">
              {oauth.userCode || 'waiting…'}
            </span>
          </div>
          <div className="mt-2 flex items-center justify-center gap-2">
            {oauth.userCode && <CopyButton text={oauth.userCode} label="Copy code" />}
            {oauth.verificationUrl && <CopyButton text={oauth.verificationUrl} label="Copy URL" />}
          </div>
          <div className="mt-3 flex items-center justify-center gap-1.5 text-[11px] text-koma-fg opacity-50">
            <BrailleSpinner size={12} />
            waiting for approval…
          </div>
        </div>
        <CancelFooter onCancel={onDone} />
      </>
    )
  }

  // codex_paste: manual access-token entry — never rendered/logged anywhere,
  // masked like a password field.
  if (oauth.phase === 'paste') {
    const submit = () => {
      if (!token.trim()) return
      req({ r: 'SubmitOAuthPaste', token: token.trim() })
    }
    return (
      <>
        <div className="flex-1 overflow-auto p-3">
          <p className="mb-3 text-[12px] text-koma-fg opacity-70">Paste the access token from Codex's CLI login.</p>
          <input
            type="password"
            autoFocus
            autoComplete="off"
            spellCheck={false}
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder="access token"
            className="h-7 w-full rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-35 focus:border-koma-grip"
          />
        </div>
        <div className="flex flex-none items-center justify-end gap-2 border-t border-koma-border px-3 py-2">
          <button
            onClick={onDone}
            className="rounded px-2.5 py-1 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
          >
            Cancel
          </button>
          <button
            onClick={submit}
            disabled={!token.trim()}
            className="rounded border border-koma-border px-2.5 py-1 text-[12px] text-koma-fg transition-colors enabled:hover:bg-koma-hover disabled:opacity-40"
          >
            Submit
          </button>
        </div>
      </>
    )
  }

  // 'success' renders nothing — the effect above navigates away before this
  // would ever paint a frame.
  return null
}
