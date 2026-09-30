import { useState, type DragEvent } from 'react'
import { GripVertical } from 'lucide-react'
import type { ComposerToken } from '../lib/composerSegments'

export function ComposerPileBar({
  tokens,
  draftLength,
  onMove,
}: {
  tokens: ComposerToken[]
  draftLength: number
  onMove: (fromStart: number, fromEnd: number, toIndex: number) => void
}) {
  const [drag, setDrag] = useState<number | null>(null)
  const [over, setOver] = useState<number | null>(null)

  if (!tokens.length) return null

  const finish = (targetIndex: number) => {
    if (drag == null || drag === targetIndex) return
    const token = tokens[drag]
    if (!token) return
    const toIndex = targetIndex <= drag ? tokens[targetIndex]?.start ?? 0 : tokens[targetIndex]?.end ?? token.end
    onMove(token.start, token.end, toIndex)
  }

  const onDragStart = (index: number, event: DragEvent) => {
    setDrag(index)
    event.dataTransfer.effectAllowed = 'move'
    event.dataTransfer.setData('text/plain', tokens[index]?.label ?? '')
  }

  return (
    <div
      className="flex flex-wrap items-center gap-1 border-b border-koma-border/60 pb-2"
      aria-label="Message attachments and references"
    >
      <span className="text-[10px] font-medium uppercase tracking-wide text-koma-dim">Pile</span>
      {tokens.map((token, index) => (
        <span
          key={`${token.kind}:${token.start}:${token.label}`}
          draggable
          onDragStart={(event) => onDragStart(index, event)}
          onDragEnd={() => {
            setDrag(null)
            setOver(null)
          }}
          onDragOver={(event) => {
            event.preventDefault()
            setOver(index)
          }}
          onDrop={(event) => {
            event.preventDefault()
            finish(index)
            setDrag(null)
            setOver(null)
          }}
          className={`flex max-w-full cursor-grab items-center gap-0.5 rounded-md border px-1.5 py-0.5 text-[11px] active:cursor-grabbing ${
            token.kind === 'fileRef'
              ? 'border-koma-accent/30 bg-koma-accent/10 text-koma-fg'
              : 'border-koma-warn/40 bg-koma-warn/15 text-koma-fg'
          } ${drag === index ? 'opacity-50' : ''} ${over === index && drag != null && drag !== index ? 'ring-1 ring-koma-accent' : ''}`}
          title="Drag to reorder in message"
        >
          <GripVertical size={11} className="flex-none opacity-50" aria-hidden />
          <span className="truncate">{token.label}</span>
        </span>
      ))}
      <span
        className="min-h-[22px] flex-1"
        onDragOver={(event) => {
          if (drag == null) return
          event.preventDefault()
          setOver(tokens.length)
        }}
        onDrop={(event) => {
          event.preventDefault()
          if (drag == null) return
          const token = tokens[drag]
          if (token) onMove(token.start, token.end, draftLength)
          setDrag(null)
          setOver(null)
        }}
      />
    </div>
  )
}
