import { useState, type ReactNode } from 'react'
import { ChevronUp, Circle, Frame, Hexagon, Minus, Square, Star, Triangle, type LucideIcon } from 'lucide-react'
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
  items: { label: string; icon?: LucideIcon; run: () => void }[]
}) {
  const [open, setOpen] = useState(false)
  return (
    <div className="relative flex flex-none items-center" onPointerLeave={() => setOpen(false)}>
      <ToolButton
        label={label}
        selected={selected}
        onClick={onClick}
      >
        {children}
      </ToolButton>
      {/* The menu opens upward from the bottom toolbar, so the chevron points up. */}
      <button
        type="button"
        title={`${label} options`}
        aria-label={`${label} options`}
        aria-haspopup="menu"
        aria-expanded={open}
        className={`flex h-6 w-3.5 flex-none items-center justify-center rounded transition ${
          open ? 'bg-koma-hover text-koma-fg' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
        }`}
        onClick={(event) => {
          event.stopPropagation()
          setOpen((current) => !current)
        }}
      >
        <ChevronUp size={12} strokeWidth={2.25} />
      </button>
      {open ? (
        // The transparent bottom padding bridges the gap to the button, so moving up into
        // the menu never leaves the hover area and closes it.
        <div className="absolute bottom-full left-0 z-40 pb-1">
          <div className="min-w-36 rounded border border-koma-border bg-koma-panel py-1 shadow-lg">
            {items.map((item) => (
              <button
                key={item.label}
                type="button"
                className="flex h-7 w-full items-center gap-2 px-2 text-left text-[12px] text-koma-fg hover:bg-koma-hover"
                onClick={() => {
                  item.run()
                  setOpen(false)
                }}
              >
                {item.icon ? <item.icon size={13} strokeWidth={2} className="flex-none text-koma-dim" /> : null}
                {item.label}
              </button>
            ))}
          </div>
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

type ShapeTool = 'rect' | 'ellipse' | 'triangle' | 'polygon' | 'star' | 'line'

const SHAPE_TOOLS: { tool: ShapeTool; label: string; icon: LucideIcon }[] = [
  { tool: 'rect', label: 'Rectangle (R)', icon: Square },
  { tool: 'ellipse', label: 'Ellipse (O)', icon: Circle },
  { tool: 'triangle', label: 'Triangle', icon: Triangle },
  { tool: 'polygon', label: 'Polygon', icon: Hexagon },
  { tool: 'star', label: 'Star', icon: Star },
  { tool: 'line', label: 'Line (L)', icon: Minus },
]

/** One button for every basic shape. It shows and reuses the shape picked last. */
export function ShapeToolFlyout({ tool, onTool }: { tool: Tool; onTool: (tool: Tool) => void }) {
  const active = SHAPE_TOOLS.find((item) => item.tool === tool)
  const [last, setLast] = useState<ShapeTool>('rect')
  const current = active?.tool ?? last
  const shown = SHAPE_TOOLS.find((item) => item.tool === current) ?? SHAPE_TOOLS[0]
  const pick = (next: ShapeTool) => {
    setLast(next)
    onTool(next)
  }
  return (
    <ToolFlyout
      label={shown.label}
      selected={!!active}
      onClick={() => pick(current)}
      items={SHAPE_TOOLS.map((item) => ({ label: item.label, icon: item.icon, run: () => pick(item.tool) }))}
    >
      <shown.icon size={15} strokeWidth={2.25} />
    </ToolFlyout>
  )
}
