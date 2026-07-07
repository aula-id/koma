import { useState } from 'react'
import type { ReactNode } from 'react'
import { ChevronRight } from 'lucide-react'

type AccordionSectionProps = {
  title: string
  defaultOpen?: boolean
  action?: ReactNode
  children?: ReactNode
}

// VSCode-style collapsible section. `action` (e.g. a + button) shows on hover.
// Header is sticky (stays visible while its section scrolls past); the body is
// height-capped and scrolls internally so a long list never shoves the other
// sections' headers out of view.
export function AccordionSection({ title, defaultOpen = true, action, children }: AccordionSectionProps) {
  const [open, setOpen] = useState(defaultOpen)
  return (
    <div className="flex flex-none flex-col">
      <div className="group sticky top-0 z-[1] flex h-[22px] flex-none items-center bg-koma-head pr-1 hover:bg-koma-hover">
        <button
          onClick={() => setOpen((o) => !o)}
          className="flex h-full flex-1 items-center gap-1 px-2 text-[11px] font-semibold uppercase tracking-wide text-koma-fg opacity-75 hover:opacity-100"
        >
          <ChevronRight
            size={14}
            strokeWidth={2}
            className={`transition-transform ${open ? 'rotate-90' : ''}`}
          />
          <span className="truncate">{title}</span>
        </button>
        {action && <div className="flex items-center opacity-0 group-hover:opacity-100">{action}</div>}
      </div>
      {open && <div className="max-h-48 overflow-y-auto pb-1">{children}</div>}
    </div>
  )
}
