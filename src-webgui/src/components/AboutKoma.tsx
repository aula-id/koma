import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import { Check, Copy, ExternalLink, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import { LICENSE_URL, REPO_URL, formatBuildInfo, readBuildInfo } from '../lib/buildInfo'

export function showAbout() {
  window.dispatchEvent(new CustomEvent('koma-about'))
}

export function AboutPanel() {
  const info = readBuildInfo(window.__komaBuild)
  const openExternal = useKoma((s) => s.openExternal)
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    if (!copied) return
    const timer = window.setTimeout(() => setCopied(false), 1500)
    return () => window.clearTimeout(timer)
  }, [copied])

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(formatBuildInfo(info))
      setCopied(true)
    } catch {
      /* clipboard unavailable */
    }
  }

  const rows: [string, string][] = [
    ['Version', info.version],
    ['Channel', info.channel],
    ['Commit', info.sha],
    ['Target', info.target],
    ['OS', info.os],
    ['Built', info.buildDate ?? 'unknown'],
  ]
  const btn =
    'flex items-center gap-1.5 rounded-md border border-koma-border px-2.5 py-1 text-[11px] text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100'

  return (
    <div className="flex max-w-md flex-col gap-4 text-koma-fg">
      <div className="flex items-center gap-3">
        <img src="koma://localhost/product-icon.png" width={48} height={48} alt="" className="rounded-md" />
        <div className="text-[18px] font-semibold">Koma</div>
      </div>
      <dl className="divide-y divide-koma-border rounded-md border border-koma-border bg-koma-panel2 text-[12px]">
        {rows.map(([label, value]) => (
          <div key={label} className="flex items-center justify-between gap-4 px-3 py-1.5">
            <dt className="text-koma-dim">{label}</dt>
            <dd className="break-all text-right">{value}</dd>
          </div>
        ))}
      </dl>
      <div className="flex flex-wrap items-center gap-2">
        <button type="button" onClick={() => void copy()} className={btn}>
          {copied ? <Check size={12} className="text-koma-accent" /> : <Copy size={12} />}
          {copied ? 'Copied' : 'Copy build info'}
        </button>
        <button type="button" onClick={() => openExternal(REPO_URL)} className={btn}>
          <ExternalLink size={12} />
          Repository
        </button>
        <button type="button" onClick={() => openExternal(LICENSE_URL)} className={btn}>
          <ExternalLink size={12} />
          Licence (Apache-2.0)
        </button>
      </div>
    </div>
  )
}

export function AboutDialog() {
  const [open, setOpen] = useState(false)

  useEffect(() => {
    const onOpen = () => setOpen(true)
    window.addEventListener('koma-about', onOpen)
    return () => window.removeEventListener('koma-about', onOpen)
  }, [])

  useEffect(() => {
    if (!open) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open])

  if (!open) return null

  return createPortal(
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center bg-black/40 p-4"
      onMouseDown={() => setOpen(false)}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="About Koma"
        onMouseDown={(event) => event.stopPropagation()}
        className="relative w-full max-w-md rounded-md border border-koma-border bg-koma-panel p-5 shadow-lg"
      >
        <button
          type="button"
          onClick={() => setOpen(false)}
          aria-label="Close"
          className="absolute right-2 top-2 rounded p-1 text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
        >
          <X size={12} />
        </button>
        <AboutPanel />
      </div>
    </div>,
    document.body,
  )
}
