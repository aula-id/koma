import { createContext, useContext, type ReactNode } from 'react'
import { createPortal } from 'react-dom'

export const panelButton = 'flex h-6 items-center justify-center gap-1 rounded px-1.5 text-koma-dim hover:bg-koma-hover hover:text-koma-fg disabled:opacity-40 disabled:pointer-events-none'
export const panelIconButton = `${panelButton} w-6 px-0`
export const panelSelect = 'h-6 w-full min-w-0 bg-koma-panel2 pl-2 text-[11px]'
export const panelRow = 'flex h-[22px] w-full items-center gap-2 px-3 text-left'

// undefined = no BottomPanel (render inline); null = slot not mounted yet (render nothing).
export const PanelActionsSlot = createContext<HTMLElement | null | undefined>(undefined)

/** Renders header actions into the BottomPanel header, or inline when mounted on its own. */
export function PanelActions({ children }: { children: ReactNode }) {
  const slot = useContext(PanelActionsSlot)
  if (slot === undefined) return <>{children}</>
  return slot ? createPortal(children, slot) : null
}

export function PanelBody({ children, className = '' }: { children: ReactNode; className?: string }) {
  return <div className={`flex min-h-0 flex-1 flex-col overflow-auto ${className}`}>{children}</div>
}

export function PanelEmpty({ icon, children }: { icon?: ReactNode; children: ReactNode }) {
  return <div className="flex min-h-[96px] flex-1 flex-col items-center justify-center gap-2 px-3 py-4 text-center text-[11px] text-koma-dim">{icon}{children}</div>
}

/** Last path segment, either separator. */
export function baseName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean)
  return parts[parts.length - 1] ?? path
}
