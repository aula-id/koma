import { useEffect, useState } from 'react'
import type { ComputerPreviewFrame, ComputerPreviewRequest, ComputerStatus } from '../types/computer'

/** One outstanding frame per visible viewer. Live frames never enter the conversation. */
export function useComputerLivePreview(status: ComputerStatus | null) {
  const [frame, setFrame] = useState<ComputerPreviewFrame | null>(null)
  const session = status?.session
  const generation = status?.generation
  const selected = status?.observation?.window.id
  const active = !!status?.enabled && !status.busy && !!status.capabilities.capture
  useEffect(() => {
    setFrame(null)
    if (!active || !session || !generation || !selected) return
    let disposed = false
    let pending: string | null = null
    let timer: ReturnType<typeof setTimeout> | undefined
    const schedule = (delay: number) => { clearTimeout(timer); if (!disposed) timer = setTimeout(request, delay) }
    const request = () => {
      if (document.hidden) { schedule(500); return }
      const id = `preview:${Date.now()}:${Math.random().toString(36).slice(2)}`
      pending = id
      const preview: ComputerPreviewRequest = { id, session, generation, window: selected }
      window.ipc?.postMessage(JSON.stringify(location.hash === '#computer-preview' ? { preview } : { t: 'req', r: 'ComputerPreview', ...preview }))
      // Label a stalled feed honestly and discard its eventual late response.
      clearTimeout(timer)
      timer = setTimeout(() => {
        if (disposed) return
        pending = null
        setFrame({ request: preview, image: null, error: 'Live preview timed out', captured_ms: Date.now() })
        schedule(1500)
      }, 12_000)
    }
    const receive = (event: Event) => {
      const next = (event as CustomEvent<ComputerPreviewFrame>).detail
      if (disposed || next.request.id !== pending || next.request.session !== session || next.request.generation !== generation || next.request.window !== selected) return
      pending = null
      setFrame(next)
      schedule(next.error ? 1500 : 500)
    }
    window.addEventListener('koma-computer-preview', receive)
    request()
    return () => { disposed = true; clearTimeout(timer); window.removeEventListener('koma-computer-preview', receive) }
  }, [active, session, generation, selected])
  return frame && frame.request.session === session && frame.request.generation === generation && frame.request.window === selected && active ? frame : null
}
