import { lazy, Suspense, type CSSProperties } from 'react'

export type EmbeddedCodeEditorProps = {
  value: string
  onChange: (value: string) => void
  /** Monaco language id. 'json' also turns on JSON diagnostics. Default 'plaintext'. */
  language?: string
  readOnly?: boolean
  /** Fixed CSS height (number = px), or `{ minLines, maxLines }` to grow with the content. */
  height?: number | string | { minLines: number; maxLines: number }
  placeholder?: string
  /** Required: Monaco's input has no visible label of its own. */
  ariaLabel: string
  /** Ctrl/Cmd+Enter inside the editor. */
  onSubmit?: () => void
  /** JSON schema for completion and squiggles; only used when language is 'json'. */
  jsonSchema?: object
  lineNumbers?: boolean
  wordWrap?: boolean
  autoFocus?: boolean
  className?: string
  testId?: string
}

const Impl = lazy(() => import('./EmbeddedCodeEditorImpl'))

export function embeddedHeightStyle(height: EmbeddedCodeEditorProps['height']): CSSProperties {
  if (height === undefined) return { height: 240 }
  if (typeof height === 'object') return { height: `${Math.round(height.minLines * 19 + 12)}px` }
  return { height }
}

export const EMBEDDED_BOX =
  'relative min-w-0 overflow-hidden rounded border border-koma-border bg-koma-bg focus-within:border-koma-grip'

/**
 * A small Monaco editor for forms and tabs. Monaco itself is lazy-loaded through the same
 * setup as the code tabs (one copy, the koma theme and mono font, live theme switching);
 * until it arrives a plain textarea with the same value and shortcuts stands in, so typing
 * is never lost. Tab indents; press Escape then Tab to move focus out (Ctrl/Cmd+M toggles
 * tab-focus mode like VS Code).
 */
export function EmbeddedCodeEditor(props: EmbeddedCodeEditorProps) {
  return (
    <Suspense fallback={<Fallback {...props} />}>
      <Impl {...props} />
    </Suspense>
  )
}

function Fallback({ value, onChange, readOnly, height, placeholder, ariaLabel, onSubmit, wordWrap, className = '', testId }: EmbeddedCodeEditorProps) {
  return (
    <div className={`${EMBEDDED_BOX} ${className}`} style={embeddedHeightStyle(height)} data-testid={testId} data-editor-state="loading">
      <textarea
        aria-label={ariaLabel}
        value={value}
        readOnly={readOnly}
        placeholder={placeholder}
        spellCheck={false}
        wrap={wordWrap ? 'soft' : 'off'}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (onSubmit && e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
            e.preventDefault()
            onSubmit()
          }
        }}
        className="absolute inset-0 h-full w-full resize-none bg-transparent px-2 py-1.5 font-mono text-[12px] leading-[19px] text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-35"
      />
    </div>
  )
}
