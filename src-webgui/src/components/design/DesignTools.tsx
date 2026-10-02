import { useState, type ReactNode } from 'react'
import { Frame, Square } from 'lucide-react'
import { FRAME_PRESETS, type FramePreset } from '../../lib/design'
import { ToolButton } from './DesignRulers'
import type { Tool } from './tabShared'

export function ToolFlyout({
  label,
  selected,
  onClick,
  children,
  items,
}: {
  label: string
  selected: boolean
  onClick: () => void
  children: ReactNode
  items: { label: string; run: () => void }[]
}) {
  const [open, setOpen] = useState(false)
  return (
    <div className="relative" onPointerLeave={() => setOpen(false)}>
      <ToolButton
        label={label}
        selected={selected}
        onClick={onClick}
      >
        {children}
      </ToolButton>
      <button
        type="button"
        title={`${label} options`}
        aria-label={`${label} options`}
        className="absolute -right-0.5 -top-0.5 h-2 w-2 rounded-full bg-koma-dim/50"
        onClick={(event) => {
          event.stopPropagation()
          setOpen((current) => !current)
        }}
      />
      {open ? (
        <div className="absolute bottom-8 left-0 z-40 min-w-36 rounded border border-koma-border bg-koma-panel py-1 shadow-lg">
          {items.map((item) => (
            <button
              key={item.label}
              type="button"
              className="flex h-7 w-full items-center px-2 text-left text-[12px] text-koma-fg hover:bg-koma-hover"
              onClick={() => {
                item.run()
                setOpen(false)
              }}
            >
              {item.label}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  )
}

export function FrameToolFlyout({
  tool,
  onTool,
  onPreset,
}: {
  tool: Tool
  onTool: (tool: Tool) => void
  onPreset: (preset: FramePreset) => void
}) {
  return (
    <ToolFlyout
      label={tool === 'section' ? 'Section (S)' : 'Frame (F)'}
      selected={tool === 'frame' || tool === 'section'}
      onClick={() => onTool(tool === 'section' ? 'section' : 'frame')}
      items={[
        { label: 'Frame', run: () => onTool('frame') },
        { label: 'Section', run: () => onTool('section') },
        ...FRAME_PRESETS.map((preset) => ({ label: `${preset.name} ${preset.width}×${preset.height}`, run: () => onPreset(preset) })),
      ]}
    >
      <Frame size={15} strokeWidth={2.25} />
    </ToolFlyout>
  )
}

export function ShapeToolFlyout({ tool, onTool }: { tool: Tool; onTool: (tool: Tool) => void }) {
  return (
    <ToolButton label="Rectangle (R)" selected={tool === 'rect' || tool === 'ellipse' || tool === 'line' || tool === 'polygon' || tool === 'star'} onClick={() => onTool('rect')}>
      <Square size={15} strokeWidth={2.25} />
    </ToolButton>
  )
}
