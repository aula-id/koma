import { useId, type ReactNode } from 'react'
import { ChevronRight } from 'lucide-react'

type AccordionSectionProps = {
  title: string
  open: boolean
  onToggle: () => void
  action?: ReactNode
  children?: ReactNode
  /** Open sections fill leftover height. Short palettes pass false so the list below can. */
  fill?: boolean
  /** Non-filling only: shrink below the content height and scroll inside when the panel is short. */
  shrink?: boolean
  /** Smallest height (px) an open filling section keeps, so a shrinking sibling cannot squeeze it out. */
  minHeight?: number
  /** Shown after the title. Pass only when greater than 0: never display "0". */
  count?: number
}

// VSCode-style collapsible section. `action` (e.g. a + button) is always shown;
// the action sets its own dimming (a wrapper here would compound with it and
// drop the label below 4.5:1). The header is an @container so actions can
// drop their text label when the sidebar is narrow (see AddBtn).
// An OPEN section FLEX-FILLS the remaining panel height (`flex-1 min-h-0`) and
// scrolls its body internally, so there's no dead gap below the last section
// and a long list never shoves the other headers out of view. A CLOSED section
// collapses to just its header (`flex-none`). Requires the parent to be a flex
// column with `min-h-0` (see ExplorePanel/ConnectorListView).
export function AccordionSection({ title, open, onToggle, action, children, fill = true, shrink = false, minHeight, count }: AccordionSectionProps) {
  const bodyId = useId()
  const body = fill ? 'min-h-0 flex-1 overflow-y-auto pb-1' : shrink ? 'min-h-0 overflow-y-auto pb-1' : 'pb-1'
  return (
    <div
      className={`flex flex-col ${open ? (fill ? 'min-h-0 flex-1' : shrink ? 'min-h-0 flex-initial' : 'flex-none') : 'flex-none'}`}
      style={open && fill && minHeight ? { minHeight } : undefined}
    >
      <div className="group @container flex h-[24px] flex-none items-center bg-koma-head pr-1 hover:bg-koma-hover">
        <button
          onClick={onToggle}
          aria-expanded={open}
          aria-controls={open ? bodyId : undefined}
          className="flex h-full flex-1 items-center gap-1 px-2 text-[11px] font-semibold uppercase text-koma-fg opacity-75 hover:opacity-100"
        >
          <ChevronRight
            size={14}
            strokeWidth={2}
            className={`transition-transform ${open ? 'rotate-90' : ''}`}
          />
          <span className="truncate">{title}</span>
          {count !== undefined && <span className="ml-1 flex-none font-normal normal-case opacity-75">{count}</span>}
        </button>
        {action && (
          <div className="flex items-center">{action}</div>
        )}
      </div>
      {open && <div id={bodyId} className={body}>{children}</div>}
    </div>
  )
}
