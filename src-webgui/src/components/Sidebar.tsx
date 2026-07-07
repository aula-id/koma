import { useState } from 'react'
import { ChevronRight } from 'lucide-react'

type SidebarProps = {
  width: number
}

// VSCode-style accordion sections. Bodies are intentionally empty for now —
// this is the scaffold; content (file list, bash panel, agent list) comes later.
const SECTIONS = ['File changed', 'Bash', 'Agents'] as const

// Collapsible side panel. Width is driven by RootLayout state.
export function Sidebar({ width }: SidebarProps) {
  // Every section starts expanded (VSCode default). Tracked by title.
  const [open, setOpen] = useState<Record<string, boolean>>(() =>
    Object.fromEntries(SECTIONS.map((s) => [s, true])),
  )
  const toggle = (title: string) => setOpen((o) => ({ ...o, [title]: !o[title] }))

  return (
    <div
      style={{ width, flexBasis: width }}
      className="flex min-w-0 flex-none flex-col overflow-hidden border-r border-koma-border bg-koma-panel"
    >
      <div className="flex h-[35px] flex-none items-center px-5 text-[11px] uppercase tracking-wider text-koma-fg opacity-60 whitespace-nowrap">
        Explorer
      </div>
      <div className="flex-1 overflow-auto">
        {SECTIONS.map((title) => (
          <AccordionSection
            key={title}
            title={title}
            open={open[title]}
            onToggle={() => toggle(title)}
          />
        ))}
      </div>
    </div>
  )
}

type AccordionSectionProps = {
  title: string
  open: boolean
  onToggle: () => void
}

function AccordionSection({ title, open, onToggle }: AccordionSectionProps) {
  return (
    <div className="flex flex-none flex-col">
      <button
        onClick={onToggle}
        className="flex h-[22px] flex-none items-center gap-1 bg-koma-head px-2 text-[11px] font-semibold uppercase tracking-wide text-koma-fg opacity-75 hover:bg-koma-hover hover:opacity-100"
      >
        <ChevronRight
          size={14}
          strokeWidth={2}
          className={`transition-transform ${open ? 'rotate-90' : ''}`}
        />
        <span className="truncate">{title}</span>
      </button>
      {open && <div className="min-h-[8px] pb-1" />}
    </div>
  )
}
