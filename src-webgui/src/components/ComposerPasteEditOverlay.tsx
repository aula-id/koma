import { useEffect, useState } from 'react'
import { motion } from 'framer-motion'
import { Check, FileText, X } from 'lucide-react'
import { LexicalMarkdownEditor } from './lexical/LexicalMarkdownEditor'

/** Inline panel above the composer — same shell as ApprovalOverlay, editable body. */
export function ComposerPasteEditOverlay({
  markerN,
  title,
  initialText,
  loading,
  onSave,
  onClose,
}: {
  markerN: number
  title: string
  initialText: string
  loading?: boolean
  onSave: (markerN: number, text: string) => void
  onClose: () => void
}) {
  const [body, setBody] = useState(initialText)

  useEffect(() => {
    setBody(initialText)
  }, [initialText, markerN])

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  return (
    <div className="pb-1">
      <motion.div
        initial={{ opacity: 0, scale: 0.97, y: 6 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ duration: 0.16, ease: 'easeOut' }}
        className="flex max-h-[40vh] w-full flex-col overflow-hidden rounded-xl border border-koma-border bg-koma-panel shadow-lg"
      >
        <div className="flex items-center gap-2 border-b border-koma-border px-4 py-2.5 text-koma-fg">
          <FileText size={16} className="flex-none text-koma-accent" />
          <span className="text-[13px] font-semibold">{title}</span>
        </div>

        <div className="min-h-0 flex-1 overflow-auto px-4 py-3">
          <div className="text-[10px] uppercase tracking-wider text-koma-fg opacity-45">Content</div>
          {loading ? (
            <p className="mt-2 text-[12px] text-koma-dim">Loading pasted text…</p>
          ) : (
            <div className="mt-1 max-h-[22vh] min-h-[120px] overflow-auto rounded border border-koma-border bg-koma-bg px-2.5 py-2">
              <LexicalMarkdownEditor
                profile="inline"
                markdown={body}
                onMarkdown={setBody}
                controlled
                className="min-h-[100px] font-mono text-[11.5px] leading-snug text-koma-fg"
                ariaLabel="Edit pasted text"
              />
            </div>
          )}
        </div>

        <div className="flex items-center justify-start gap-2 border-t border-koma-border px-4 py-2.5">
          <button
            type="button"
            disabled={loading}
            onClick={() => {
              onSave(markerN, body)
              onClose()
            }}
            className="flex items-center gap-1.5 rounded-md border border-koma-accent bg-koma-accent/15 px-3 py-1.5 text-[12px] text-koma-accent transition-colors hover:bg-koma-accent/25 disabled:opacity-40"
          >
            <Check size={13} className="flex-none" />
            Save
          </button>
          <button
            type="button"
            onClick={onClose}
            className="flex items-center gap-1.5 rounded-md border border-koma-border bg-koma-panel px-3 py-1.5 text-[12px] text-koma-fg opacity-80 transition-colors hover:bg-koma-hover hover:opacity-100"
          >
            <X size={13} className="flex-none" />
            Cancel
          </button>
        </div>
      </motion.div>
    </div>
  )
}
