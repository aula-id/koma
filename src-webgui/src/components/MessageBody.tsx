import { createContext, memo, useContext, useEffect, useRef, useState } from 'react'
import { Streamdown } from 'streamdown'
import { komaCode } from './komaShiki'
import { useKoma } from '../store/koma'
import { luminance } from '../lib/luminance'

// Scroll root for IntersectionObserver — ChatView's transcript scroller, not
// the window. Without this, off-screen bubbles inside the nested overflow
// still look "visible" to the viewport and all run Streamdown/Shiki on attach.
export const ChatScrollRootContext = createContext<HTMLElement | null>(null)

function scheduleIdle(cb: () => void, timeoutMs: number): () => void {
  const w = window as Window & {
    requestIdleCallback?: (fn: () => void, opts?: { timeout: number }) => number
    cancelIdleCallback?: (id: number) => void
  }
  if (typeof w.requestIdleCallback === 'function') {
    const id = w.requestIdleCallback(cb, { timeout: timeoutMs })
    return () => w.cancelIdleCallback?.(id)
  }
  const id = window.setTimeout(cb, Math.min(timeoutMs, 48))
  return () => window.clearTimeout(id)
}

// Streaming-safe markdown + code renderer. Wraps Streamdown (Vercel) which is
// a react-markdown-compatible renderer purpose-built for the partial/
// unterminated-token case: `parseIncompleteMarkdown` runs remend's repair
// pipeline on a copy of the raw string before parsing, so a half-written
// **bold, ```fence or [link mid-stream never crashes or flashes. Per-block
// memoization (marked lexer split + React.memo) keeps long transcripts from
// re-rendering wholesale on each token.
//
// Code fences are highlighted by `@streamdown/code` (Shiki), which uses
// Shiki's JavaScript regex engine (createJavaScriptRegexEngine) rather than
// the oniguruma WASM engine — so nothing has to ship/serve a `.wasm` blob
// through the offline `koma://` protocol. Copy affordance is built in.
//
// Feed the FULL accumulated string as `text`, never deltas — Streamdown
// expects the complete current markdown as children (block memoization +
// repair both assume it). Memoized per message so sibling bubbles don't
// re-render as the live bubble grows.
//
// Committed history starts as plain text and upgrades to Streamdown only once
// the bubble intersects the chat scroller (idle-scheduled) — a fat Snapshot
// must not mount hundreds of Shiki highlighters on the first frame.
// While `streaming`, Streamdown is used immediately and the visible text is
// THROTTLED (~50ms / rAF) so Shiki + repair don't run on every token.
export const MessageBody = memo(function MessageBody({
  text,
  streaming = false,
}: {
  text: string
  streaming?: boolean
}) {
  // Streamdown/komaShiki tokenize at RENDER time into static colored spans —
  // a palette flip alone (CSS var repaint) doesn't re-tokenize an
  // already-rendered code block. Subscribing to the live koma bg here (same
  // store slice `applyPaletteVars` keeps in sync with `--koma-bg`, see
  // store/koma.ts) makes MessageBody re-render on a theme flip, and passing a
  // freshly-computed `shikiTheme` array down forces Streamdown to recompute
  // its shiki context + re-highlight (see komaShiki.ts's `getThemes` comment
  // for why that prop change is what actually cascades into a re-tokenize).
  const bg = useKoma((s) => s.palette.bg)
  const codeTheme = luminance(bg) >= 0.5 ? 'github-light' : 'github-dark'
  const scrollRoot = useContext(ChatScrollRootContext)

  const hostRef = useRef<HTMLDivElement>(null)
  const [rich, setRich] = useState(streaming)
  const [shown, setShown] = useState(text)
  const pendingRef = useRef(text)
  const rafRef = useRef(0)
  const lastPaintRef = useRef(0)

  useEffect(() => {
    if (streaming) {
      setRich(true)
      return
    }
    const el = hostRef.current
    if (!el) return
    let cancelUpgrade: (() => void) | null = null
    const io = new IntersectionObserver(
      ([entry]) => {
        if (!entry?.isIntersecting) return
        io.disconnect()
        cancelUpgrade = scheduleIdle(() => setRich(true), 400)
      },
      { root: scrollRoot, rootMargin: '160px 0px', threshold: 0 },
    )
    io.observe(el)
    return () => {
      io.disconnect()
      cancelUpgrade?.()
    }
  }, [streaming, scrollRoot])

  useEffect(() => {
    if (!streaming) {
      if (rafRef.current) {
        cancelAnimationFrame(rafRef.current)
        rafRef.current = 0
      }
      setShown(text)
      pendingRef.current = text
      return
    }
    pendingRef.current = text
    const tick = (now: number) => {
      rafRef.current = 0
      // ~20 Hz max while streaming; always paint if we lagged > 50ms.
      if (now - lastPaintRef.current >= 50) {
        lastPaintRef.current = now
        setShown(pendingRef.current)
      } else if (pendingRef.current !== shown) {
        rafRef.current = requestAnimationFrame(tick)
      }
    }
    if (!rafRef.current) {
      rafRef.current = requestAnimationFrame(tick)
    }
    return () => {
      if (rafRef.current) {
        cancelAnimationFrame(rafRef.current)
        rafRef.current = 0
      }
    }
  }, [text, streaming, shown])

  const body = streaming ? shown : text

  if (!rich) {
    return (
      <div ref={hostRef} className="koma-md whitespace-pre-wrap break-words text-[13px] text-koma-fg">
        {body}
      </div>
    )
  }

  return (
    <div ref={hostRef}>
      <Streamdown
        className="koma-md"
        mode={streaming ? 'streaming' : 'static'}
        // Repair-before-parse while live; off on the committed final frame.
        parseIncompleteMarkdown={streaming}
        // Trimmed Shiki highlighter (JS regex engine, no WASM; ~16 langs) +
        // copy button. See komaShiki.ts for why we don't use the stock plugin.
        plugins={{ code: komaCode }}
        shikiTheme={[codeTheme, codeTheme]}
        // Line numbers are noise inside a chat bubble; drop them.
        lineNumbers={false}
        // Keep the code copy button; suppress the table/mermaid toolbars — the
        // 1:1 TUI grammar has no such affordances.
        controls={{ code: { copy: true, download: false }, table: false, mermaid: false }}
      >
        {body}
      </Streamdown>
    </div>
  )
})
