import { memo } from 'react'
import { Streamdown } from 'streamdown'
import { code } from '@streamdown/code'

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
  return (
    <Streamdown
      className="koma-md"
      mode={streaming ? 'streaming' : 'static'}
      // Repair-before-parse while live; off on the committed final frame.
      parseIncompleteMarkdown={streaming}
      // Shiki highlighter (JS regex engine, no WASM) + copy button.
      plugins={{ code }}
      shikiTheme={['github-dark', 'github-dark']}
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
