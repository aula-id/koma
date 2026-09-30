import { useEffect, useState } from 'react'
import { motion } from 'framer-motion'
import { Check, X } from 'lucide-react'
import { LexicalMarkdownEditor } from './lexical/LexicalMarkdownEditor'

export function ComposerPasteEditOverlay({
  markerN,
  title,
  initialText,
  onSave,
  onClose,
}: {
  markerN: number
  title: string
  initialText: string
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
    <div className="absolute inset-0 z-[60] flex items-center justify-center bg-koma-bg/70 p-4" onMouseDown={onClose}>
      <motion.div
        initial={{ opacity: 0, scale: 0.98 }}
        animate={{ opacity: 1, scale: 1 }}
        className="flex max-h-[min(80vh,640px)] w-full max-w-2xl flex-col rounded-xl border border-koma-border bg-koma-panel shadow-xl"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-koma-border px-4 py-3">
          <h2 className="text-[13px] font-medium text-koma-fg">{title}</h2>
          <button type="button" className="rounded-lg p-1 text-koma-dim hover:bg-koma-hover" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </div>
        <div className="min-h-[200px] flex-1 overflow-y-auto px-4 py-3">
          <LexicalMarkdownEditor
            profile="inline"
            markdown={body}
            onMarkdown={setBody}
            controlled
            className="min-h-[180px] text-[13px] text-koma-fg"
            ariaLabel="Edit pasted text"
          />
        </div>
        <div className="flex justify-end gap-2 border-t border-koma-border px-4 py-3">
          <button
            type="button"
            className="rounded-lg border border-koma-border px-3 py-1.5 text-[12px] text-koma-fg hover:bg-koma-hover"
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="button"
            className="flex items-center gap-1 rounded-lg bg-koma-accent px-3 py-1.5 text-[12px] text-koma-bg hover:opacity-90"
            onClick={() => {
              onSave(markerN, body)
              onClose()
            }}
          >
            <Check size={14} />
            Save
          </button>
        </div>
      </motion.div>
    </div>
  )
}
