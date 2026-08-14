import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from 'react'
import { motion } from 'framer-motion'
import { Check, Lock, X } from 'lucide-react'
import { CMD_SEARCH_WIDTH } from './Titlebar'

type RemotePasswordPromptProps = {
  active: boolean
  target?: string | null
  onSubmit: (password: string) => void
  onCancel: () => void
}

/**
 * Full-screen overlay with a narrow top pill bar for SSH password entry.
 * Follows the RenameOverlay / OmniSearchPalette pattern: absolute inset-0
 * backdrop, narrow centered bar at top, Esc/click-cancel, Enter/✓ submit.
 */
export function RemotePasswordPrompt({
  active,
  target,
  onSubmit,
  onCancel,
}: RemotePasswordPromptProps) {
  const [password, setPassword] = useState('')
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (active) {
      // Delay one tick so the motion div is in the DOM before focusing.
      requestAnimationFrame(() => inputRef.current?.focus())
    } else {
      setPassword('')
    }
    return () => setPassword('')
  }, [active])

  if (!active) return null

  const submit = (event: FormEvent) => {
    event.preventDefault()
    if (!password.trim()) return
    const value = password
    setPassword('')
    onSubmit(value)
  }

  const cancel = () => {
    setPassword('')
    onCancel()
  }

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault()
      cancel()
    }
  }

  return (
    <div className="absolute inset-0 z-[70]" onMouseDown={cancel}>
      <motion.div
        initial={{ opacity: 0, y: -4 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.12, ease: 'easeOut' }}
        className={`mx-auto mt-[5px] ${CMD_SEARCH_WIDTH}`}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <form
          onSubmit={submit}
          className="flex h-[22px] items-center gap-1.5 rounded-md border border-koma-border bg-koma-panel px-2.5 shadow-xl"
        >
          <Lock size={12} className="flex-none text-koma-dim" />
          <input
            ref={inputRef}
            type="password"
            autoComplete="current-password"
            aria-label="SSH password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Enter password…"
            className="w-full bg-transparent text-[12px] text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-40"
          />
          <button
            type="submit"
            disabled={!password.trim()}
            title="Submit"
            aria-label="Submit password"
            className="flex h-4 w-4 flex-none items-center justify-center rounded text-koma-fg opacity-60 transition-colors hover:text-emerald-500 hover:opacity-100 disabled:cursor-not-allowed disabled:opacity-30"
          >
            <Check size={13} />
          </button>
          <button
            type="button"
            onClick={cancel}
            title="Cancel"
            aria-label="Cancel"
            className="flex h-4 w-4 flex-none items-center justify-center rounded text-koma-fg opacity-60 transition-colors hover:text-red-500 hover:opacity-100"
          >
            <X size={13} />
          </button>
        </form>
        {target && (
          <div className="mt-1 text-center text-[11px] text-koma-fg opacity-50">
            Password for {target}
          </div>
        )}
      </motion.div>
    </div>
  )
}
