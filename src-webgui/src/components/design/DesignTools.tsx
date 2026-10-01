import { useState, type ReactNode } from 'react'
import { Circle, Frame, Hexagon, Minus, Square, Star } from 'lucide-react'
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
  const current = tool === 'line' || tool === 'ellipse' || tool === 'polygon' || tool === 'star' ? tool : 'rect'
  return (
    <ToolFlyout
      label={current === 'ellipse' ? 'Ellipse (O)' : current === 'line' ? 'Line (L)' : current === 'polygon' ? 'Polygon' : current === 'star' ? 'Star' : 'Rectangle (R)'}
      selected={tool === 'rect' || tool === 'ellipse' || tool === 'line' || tool === 'polygon' || tool === 'star'}
      onClick={() => onTool(current)}
      items={[
        { label: 'Rectangle', run: () => onTool('rect') },
        { label: 'Line', run: () => onTool('line') },
        { label: 'Ellipse', run: () => onTool('ellipse') },
        { label: 'Polygon', run: () => onTool('polygon') },
        { label: 'Star', run: () => onTool('star') },
      ]}
    >
      {current === 'ellipse' ? <Circle size={15} strokeWidth={2.25} /> : current === 'line' ? <Minus size={15} strokeWidth={2.25} /> : current === 'star' ? <Star size={15} strokeWidth={2.25} /> : current === 'polygon' ? <Hexagon size={15} strokeWidth={2.25} /> : <Square size={15} strokeWidth={2.25} />}
    </ToolFlyout>
  )
}
