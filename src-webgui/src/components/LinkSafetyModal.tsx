import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import { Check, Copy, ExternalLink, X } from 'lucide-react'
import type { LinkSafetyConfig, LinkSafetyModalProps } from 'streamdown'
import { useKoma } from '../store/koma'

/** Streamdown's stock confirm is a shadcn/pink card. Swap it for Koma chrome. */
export function LinkSafetyModal({ isOpen, onClose, url }: LinkSafetyModalProps) {
  const openExternal = useKoma((s) => s.openExternal)
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    if (!isOpen) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [isOpen, onClose])

  useEffect(() => {
    if (!isOpen) setCopied(false)
  }, [isOpen])

  if (!isOpen) return null

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(url)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    } catch {
      /* clipboard unavailable — URL stays selectable */
    }
  }

  const open = () => {
    openExternal(url)
    onClose()
  }

  return createPortal(
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center bg-black/40 p-4"
      onMouseDown={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Open external link"
        onMouseDown={(event) => event.stopPropagation()}
        className="flex w-full max-w-sm flex-col gap-3 rounded-md border border-koma-border bg-koma-panel p-3 shadow-lg"
      >
        <div className="flex items-center gap-2 text-[12px] text-koma-fg">
          <ExternalLink size={14} className="flex-none text-koma-accent" />
          <span className="min-w-0 flex-1 font-semibold">Open external link?</span>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="rounded p-1 text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
          >
            <X size={12} />
          </button>
        </div>
        <div className="text-[11px] text-koma-dim">You're about to visit an external website.</div>
        <div className="break-all rounded border border-koma-border bg-koma-bg px-2 py-1.5 text-[11px] text-koma-accent">
          {url}
        </div>
        <div className="flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={() => void copy()}
            className="flex items-center gap-1.5 rounded-md border border-koma-border px-2.5 py-1 text-[11px] text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100"
          >
            {copied ? <Check size={12} className="text-koma-accent" /> : <Copy size={12} />}
            {copied ? 'Copied' : 'Copy'}
          </button>
          <button
            type="button"
            autoFocus
            onClick={open}
            className="flex items-center gap-1.5 rounded-md border border-koma-accent bg-koma-accent/15 px-2.5 py-1 text-[11px] text-koma-accent hover:bg-koma-accent/25"
          >
            <ExternalLink size={12} />
            Open
          </button>
        </div>
      </div>
    </div>,
    document.body,
  )
}

export const komaLinkSafety: LinkSafetyConfig = {
  enabled: true,
  renderModal: (props) => <LinkSafetyModal {...props} />,
}
