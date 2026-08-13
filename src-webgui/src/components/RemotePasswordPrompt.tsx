import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from 'react'

type RemotePasswordPromptProps = {
  active: boolean
  target?: string | null
  onSubmit: (password: string) => void
  onCancel: () => void
  compact?: boolean
}

/** Password stays component-local and is cleared on every exit path. */
export function RemotePasswordPrompt({
  active,
  target,
  onSubmit,
  onCancel,
  compact = false,
}: RemotePasswordPromptProps) {
  const [password, setPassword] = useState('')
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (active) inputRef.current?.focus()
    else setPassword('')
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
    <form
      onSubmit={submit}
      className={`flex flex-col gap-2 ${compact ? 'mt-2' : 'w-[280px]'}`}
    >
      <label className="text-[12px] text-koma-fg opacity-70">
        Password{target ? ` for ${target}` : ''}
      </label>
      <input
        ref={inputRef}
        autoFocus
        type="password"
        autoComplete="current-password"
        aria-label="SSH password"
        value={password}
        onChange={(event) => setPassword(event.target.value)}
        onKeyDown={handleKeyDown}
        className="w-full rounded border border-koma-border bg-koma-panel px-2 py-1.5 text-koma-fg outline-none focus:border-koma-accent"
      />
      <div className="flex justify-end gap-2">
        <button
          type="button"
          onClick={cancel}
          className="rounded border border-koma-border px-2.5 py-1 text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
        >
          Cancel
        </button>
        <button
          type="submit"
          disabled={!password.trim()}
          className="rounded bg-koma-accent px-2.5 py-1 text-koma-bg disabled:cursor-not-allowed disabled:opacity-40"
        >
          Connect
        </button>
      </div>
    </form>
  )
}
