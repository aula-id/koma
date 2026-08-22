import type { ReactNode } from 'react'
import { ChevronRight } from 'lucide-react'

type AccordionSectionProps = {
  title: string
  open: boolean
  onToggle: () => void
  action?: ReactNode
  children?: ReactNode
}

// VSCode-style collapsible section. `action` (e.g. a + button) shows on hover.
// An OPEN section FLEX-FILLS the remaining panel height (`flex-1 min-h-0`) and
// scrolls its body internally, so there's no dead gap below the last section
// and a long list never shoves the other headers out of view. A CLOSED section
// collapses to just its header (`flex-none`). Requires the parent to be a flex
// column with `min-h-0` (see ExplorePanel/ConnectorListView).
export function AccordionSection({ title, open, onToggle, action, children }: AccordionSectionProps) {
  return (
    <div className={`flex flex-col ${open ? 'min-h-0 flex-1' : 'flex-none'}`}>
      <div className="group flex h-[22px] flex-none items-center bg-koma-head pr-1 hover:bg-koma-hover">
        <button
          onClick={onToggle}
          className="flex h-full flex-1 items-center gap-1 px-2 text-[11px] font-semibold uppercase tracking-wide text-koma-fg opacity-75 hover:opacity-100"
        >
          <ChevronRight
            size={14}
            strokeWidth={2}
            className={`transition-transform ${open ? 'rotate-90' : ''}`}
          />
          <span className="truncate">{title}</span>
        </button>
        {action && (
          <div className="flex items-center opacity-70 group-hover:opacity-100">{action}</div>
        )}
      </div>
      {open && <div className="min-h-0 flex-1 overflow-y-auto pb-1">{children}</div>}
    </div>
  )
}
