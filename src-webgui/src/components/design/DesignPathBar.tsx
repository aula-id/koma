import type { ReactNode } from 'react'
import {
  CornerRightDown,
  FlipHorizontal2,
  FlipVertical2,
  Link,
  Magnet,
  Merge,
  MousePointer2,
  PenTool,
  Plus,
  Route,
  Spline,
  Trash2,
  Unlink,
} from 'lucide-react'
import type { HandleMode } from '../../lib/design/vectorEdit'

export type PathBarAction =
  | 'draw'
  | 'move'
  | 'bend'
  | 'add'
  | 'delete'
  | 'merge'
  | 'join'
  | 'separate'
  | 'corner'
  | 'curve'
  | 'snap'
  | 'flipH'
  | 'flipV'
  | { handles: HandleMode }

/**
 * Penpot's node toolbar for path editing, floating at the bottom of the canvas while a
 * vector is being edited. Every button names its shortcut.
 */
export function DesignPathBar({
  mode,
  snap,
  handleMode,
  nodes,
  onAction,
}: {
  /** Draw adds points with the pen, move picks and drags nodes, bend curves whatever you drag. */
  mode: 'draw' | 'move' | 'bend'
  snap: boolean
  /** The handle mode shared by the selected nodes, or null when mixed or none selected. */
  handleMode: HandleMode | null
  /** How many nodes are selected; actions that need some are disabled without them. */
  nodes: number
  onAction: (action: PathBarAction) => void
}) {
  const button = (title: string, action: PathBarAction, icon: ReactNode, options: { on?: boolean; disabled?: boolean } = {}) => (
    <button
      type="button"
      title={title}
      aria-label={title}
      aria-pressed={options.on}
      disabled={options.disabled}
      onClick={() => onAction(action)}
      className={`flex h-7 w-7 flex-none items-center justify-center rounded disabled:opacity-30 ${
        options.on ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
      }`}
    >
      {icon}
    </button>
  )
  const divider = <span className="mx-0.5 h-5 w-px flex-none bg-koma-border" />
  const none = nodes === 0
  return (
    <div
      data-design-ui=""
      className="pointer-events-auto absolute bottom-3 left-1/2 z-40 flex -translate-x-1/2 items-center gap-0.5 rounded-md border border-koma-border bg-koma-panel p-0.5 shadow-lg"
      onPointerDown={(event) => event.stopPropagation()}
      onDoubleClick={(event) => event.stopPropagation()}
    >
      {button('Draw nodes (P)', 'draw', <PenTool size={14} />, { on: mode === 'draw' })}
      {button('Move nodes (M)', 'move', <MousePointer2 size={14} />, { on: mode === 'move' })}
      {button('Bend: drag a line to curve it, click a node to round or sharpen it (B)', 'bend', <Spline size={14} />, { on: mode === 'bend' })}
      {divider}
      {button('Add node (Shift+=)', 'add', <Plus size={14} />, { disabled: none })}
      {button('Delete node (Del)', 'delete', <Trash2 size={14} />, { disabled: none })}
      {divider}
      {button('Merge nodes (⌘J)', 'merge', <Merge size={14} />, { disabled: nodes < 2 })}
      {button('Join nodes (J)', 'join', <Link size={14} />, { disabled: nodes < 2 })}
      {button('Separate nodes (K)', 'separate', <Unlink size={14} />, { disabled: none })}
      {divider}
      {button('Make corner (X)', 'corner', <CornerRightDown size={14} />, { disabled: none })}
      {button('Make curve (C)', 'curve', <Route size={14} />, { disabled: none })}
      <div className="flex items-center rounded border border-koma-border" role="group" aria-label="Handle type">
        {(['mirror', 'aligned', 'independent'] as const).map((mode) => (
          <button
            key={mode}
            type="button"
            title={mode === 'mirror' ? 'Mirrored handles' : mode === 'aligned' ? 'Aligned handles' : 'Independent handles'}
            aria-pressed={handleMode === mode}
            disabled={none}
            onClick={() => onAction({ handles: mode })}
            className={`h-6 px-1.5 text-[11px] disabled:opacity-30 ${
              handleMode === mode ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
            }`}
          >
            {mode === 'mirror' ? 'Mirror' : mode === 'aligned' ? 'Align' : 'Free'}
          </button>
        ))}
      </div>
      {divider}
      {button('Flip horizontally (Shift+H)', 'flipH', <FlipHorizontal2 size={14} />, { disabled: nodes < 2 })}
      {button('Flip vertically (Shift+V)', 'flipV', <FlipVertical2 size={14} />, { disabled: nodes < 2 })}
      {button("Snap to nodes (⌘')", 'snap', <Magnet size={14} />, { on: snap })}
    </div>
  )
}
