import { useState } from 'react'
import { Bot, Terminal, Loader2, Check, CircleX, CircleSlash, type LucideIcon } from 'lucide-react'
import { AccordionSection } from '../AccordionSection'
import { Empty } from './helpers'
import { useKoma } from '../../store/koma'

// Shared status -> icon/tone map for both the Agents and Bash rows. Mirrors
// the TUI's run-state grammar: running = live/spinning, done = settled-good,
// error = settled-bad, killed = settled-neutral (dimmed, no color signal).
const STATUS_ICON: Record<string, LucideIcon> = {
  running: Loader2,
  done: Check,
  error: CircleX,
  killed: CircleSlash,
}

const STATUS_TONE: Record<string, string> = {
  running: 'text-koma-accent',
  done: 'text-koma-success',
  error: 'text-koma-error',
  killed: 'text-koma-dim opacity-60',
}

function StatusBadge({ status }: { status: string }) {
  const Icon = STATUS_ICON[status] ?? CircleSlash
  const tone = STATUS_TONE[status] ?? 'text-koma-dim opacity-60'
  return (
    <span className={`flex-none ${tone}`} title={status}>
      <Icon size={13} strokeWidth={2} className={status === 'running' ? 'animate-spin' : ''} />
    </span>
  )
}

export function ExplorePanel() {
  const [open, setOpen] = useState({ files: true, bash: true, agents: true })
  const subagents = useKoma((s) => s.session.subagents)
  const bash = useKoma((s) => s.session.bash)

  return (
    <div className="h-full overflow-auto">
      <AccordionSection
        title="File changed"
        open={open.files}
        onToggle={() => setOpen((s) => ({ ...s, files: !s.files }))}
      >
        <Empty>No changes</Empty>
      </AccordionSection>
      <AccordionSection
        title={`Bash · ${bash.length}`}
        open={open.bash}
        onToggle={() => setOpen((s) => ({ ...s, bash: !s.bash }))}
      >
        {bash.length === 0 ? (
          <Empty>No bash sessions</Empty>
        ) : (
          bash.map((b) => (
            <div key={b.id} className="flex min-h-[30px] items-center gap-2.5 px-3 py-1 hover:bg-koma-hover">
              <Terminal size={13} className="flex-none text-koma-fg opacity-45" />
              <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-koma-fg">{b.cmd}</span>
              <StatusBadge status={b.status} />
            </div>
          ))
        )}
      </AccordionSection>
      <AccordionSection
        title={`Agents · ${subagents.length}`}
        open={open.agents}
        onToggle={() => setOpen((s) => ({ ...s, agents: !s.agents }))}
      >
        {subagents.length === 0 ? (
          <Empty>No agents</Empty>
        ) : (
          subagents.map((a, i) => (
            <div key={`${a.name}-${i}`} className="flex min-h-[30px] items-center gap-2.5 px-3 py-1 hover:bg-koma-hover">
              <Bot size={13} className="flex-none text-koma-fg opacity-45" />
              <span className="min-w-0 flex-1 truncate text-[13px] text-koma-fg">{a.name}</span>
              <StatusBadge status={a.status} />
            </div>
          ))
        )}
      </AccordionSection>
    </div>
  )
}
