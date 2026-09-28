import { useEffect, useState } from 'react'
import { ComputerPreview } from './ComputerPreview'
import type { ComputerStatus } from '../types/computer'

/** Native floating window: the shared frame and hover display picker. */
export function ComputerViewer() {
  const [status, setStatus] = useState<ComputerStatus | null>(window.__komaComputerInitial ?? null)
  useEffect(() => {
    setStatus(window.__komaComputerInitial ?? null)
    const receive = (event: Event) => setStatus((event as CustomEvent<ComputerStatus>).detail)
    const apply = (palette: Record<string, string>) => {
      for (const key of ['bg', 'fg', 'dim', 'accent', 'panel', 'warn', 'success', 'info', 'error']) {
        const color = palette[key]
        if (typeof color === 'string' && /^#[0-9a-fA-F]{6}$/.test(color)) document.documentElement.style.setProperty(`--koma-${key}`, color)
      }
    }
    apply(window.__komaComputerPalette ?? {})
    const palette = (event: Event) => apply((event as CustomEvent<Record<string, string>>).detail)
    window.addEventListener('koma-computer-palette', palette)
    window.addEventListener('koma-computer', receive)
    return () => { window.removeEventListener('koma-computer', receive); window.removeEventListener('koma-computer-palette', palette) }
  }, [])
  const control = (action: 'windows' | 'select', window?: string) => windowControl(action, window)
  return <main className="h-screen overflow-hidden bg-koma-bg"><ComputerPreview status={status} control={control} /></main>
}

function windowControl(action: string, selectedWindow?: string) {
  window.ipc?.postMessage(JSON.stringify({ action, window: selectedWindow }))
}
