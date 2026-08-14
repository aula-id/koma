import { useEffect, useRef, useState, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import { ChevronDown, FolderOpen, Link2 } from 'lucide-react'
import { useKoma } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'

// Track the trigger button's viewport rect while the menu is open, so the
// menu can render in a body portal (fixed positioning) that no `overflow`
// ancestor can clip. Mirrors panels/form.tsx's `useAnchorRect`.
function useAnchorRect<T extends HTMLElement>(open: boolean, ref: RefObject<T | null>) {
  const [rect, setRect] = useState<DOMRect | null>(null)
  useEffect(() => {
    if (!open) {
      setRect(null)
      return
    }
    const update = () => {
      if (ref.current) setRect(ref.current.getBoundingClientRect())
    }
    update()
    window.addEventListener('scroll', update, true)
    window.addEventListener('resize', update)
    return () => {
      window.removeEventListener('scroll', update, true)
      window.removeEventListener('resize', update)
    }
  }, [open, ref])
  return rect
}

type NewSessionMenuProps = {
  afterPick?: () => void
  className?: string
}

const menuWidth = 240

// The chevron segment of the split "+ New session" button. Always shows:
//   - "New session" (opens folder picker)
//   - "New session + close current" (only when a session is attached)
//   - Remote host list (when any hosts are saved)
export function NewSessionMenu({ afterPick, className = '' }: NewSessionMenuProps) {
  const req = useKoma((s) => s.req)
  const remoteHosts = useKoma((s) => s.remoteHosts)
  const remoteState = useKoma((s) => s.remoteState)
  const startSwitching = useKoma((s) => s.startSwitching)
  const attachedId = useKoma((s) => s.session.id)
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLButtonElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const rect = useAnchorRect(open, ref)

  useEffect(() => {
    if (!open) return
    const onDoc = (e: MouseEvent) => {
      const t = e.target as Node
      if (ref.current?.contains(t) || menuRef.current?.contains(t)) return
      setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    window.addEventListener('mousedown', onDoc)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onDoc)
      window.removeEventListener('keydown', onKey)
    }
  }, [open])

  // Always render the chevron button so the dropdown is always available.

  const pick = (kill: boolean) => {
    req(kill ? { r: 'NewSession', kill: true } : { r: 'NewSession' })
    setOpen(false)
    afterPick?.()
  }

  const openFolder = () => {
    req({ r: 'NewSession', folder: true })
    setOpen(false)
    afterPick?.()
  }

  const connectRemote = (hostId: string, name: string) => {
    if (remoteState.state !== 'disconnected' && remoteState.state !== 'error') {
      // Already connecting/connected — let the user know why nothing happened.
      const s = useKoma.getState()
      const seq = s.ui.toastSeq + 1
      useKoma.setState((prev) => ({
        ui: { ...prev.ui, toastSeq: seq, toast: { id: seq, text: `Already ${remoteState.state.replace('_', ' ')}`, kind: 'error' } },
      }))
      return
    }
    startSwitching(`remote ${name}`)
    req({ r: 'ConnectRemoteHost', hostId })
    setOpen(false)
    afterPick?.()
  }

  const menuStyle = rect
    ? {
        position: 'fixed' as const,
        top: rect.bottom + 4,
        left: Math.max(8, rect.right - menuWidth),
        width: menuWidth,
        zIndex: 80,
      }
    : { display: 'none' }

  return (
    <span className={`flex flex-none items-center ${className}`}>
      <span className="mx-1.5 h-3 w-px flex-none bg-koma-border" />
      <button
        ref={ref}
        onClick={(e) => {
          e.stopPropagation()
          setOpen((o) => !o)
        }}
        aria-label="New session options"
        title="New session options"
        className="flex-none rounded p-0.5 text-koma-fg opacity-60 transition-opacity hover:opacity-100"
      >
        <ChevronDown size={12} className="flex-none" />
      </button>
      {open &&
        rect &&
        createPortal(
          <div
            ref={menuRef}
            style={menuStyle}
            className="overflow-hidden rounded-md border border-koma-border bg-koma-panel py-1 shadow-sm"
          >
            {/* Local session options — always visible */}
            <MenuItem onClick={openFolder} icon={<FolderOpen size={13} />}>
              New session
            </MenuItem>
            {attachedId && (
              <MenuItem onClick={() => pick(true)}>New session + close current</MenuItem>
            )}
            {/* Remote hosts — always visible when any are saved */}
            {remoteHosts.length > 0 && (
              <>
                <div className="my-1 mx-2 h-px bg-koma-border" />
                <div className="px-2.5 py-1 text-[10px] font-medium text-koma-dim uppercase tracking-wider">
                  Remote
                </div>
                {remoteHosts.map((host) => (
                  <MenuItem
                    key={host.id}
                    onClick={() => connectRemote(host.id, host.name)}
                    disabled={remoteState.state !== 'disconnected' && remoteState.state !== 'error'}
                    icon={
                      remoteState.hostId === host.id && remoteState.state !== 'error' && remoteState.state !== 'disconnected'
                        ? <BrailleSpinner size={13} />
                        : <Link2 size={13} />
                    }
                  >
                    <span className="font-medium">{host.name}</span>
                    <span className="ml-1 text-koma-dim">
                      {host.user}@{host.host}
                    </span>
                  </MenuItem>
                ))}
              </>
            )}
          </div>,
          document.body,
        )}
    </span>
  )
}

function MenuItem({
  onClick,
  icon,
  disabled = false,
  children,
}: {
  onClick: () => void
  icon?: React.ReactNode
  disabled?: boolean
  children: React.ReactNode
}) {
  return (
    <button
      disabled={disabled}
      onMouseDown={(e) => {
        e.preventDefault()
        if (!disabled) onClick()
      }}
      className="flex w-full items-center gap-1.5 px-2.5 py-1.5 text-left text-[12px] text-koma-fg opacity-80 transition-colors hover:bg-koma-hover hover:opacity-100 disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent"
    >
      {icon && <span className="flex-none text-koma-dim">{icon}</span>}
      {children}
    </button>
  )
}
