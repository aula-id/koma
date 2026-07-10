import { memo } from 'react'
import { Streamdown } from 'streamdown'
import { komaCode } from './komaShiki'
import { useKoma } from '../store/koma'
import { luminance } from '../lib/luminance'

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
  return (
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
      {text}
    </Streamdown>
  )
})
