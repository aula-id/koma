import { useState } from 'react'
import { AccordionSection } from '../AccordionSection'
import { Empty } from './helpers'
import { useKoma } from '../../store/koma'

const STATUS_TONE: Record<string, string> = {
  running: 'text-amber-500',
  done: 'text-emerald-500',
  killed: 'text-koma-fg opacity-50',
  error: 'text-red-500',
}

function StatusBadge({ status }: { status: string }) {
  return (
    <span className={`flex-none text-[10px] uppercase tracking-wide ${STATUS_TONE[status] ?? 'opacity-50'}`}>
      {status}
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
        title="Bash"
        open={open.bash}
        onToggle={() => setOpen((s) => ({ ...s, bash: !s.bash }))}
      >
        {bash.length === 0 ? (
          <Empty>No bash sessions</Empty>
        ) : (
          bash.map((b) => (
            <div key={b.id} className="flex items-center justify-between gap-2 px-5 py-1.5">
              <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-koma-fg">{b.cmd}</span>
              <StatusBadge status={b.status} />
            </div>
          ))
        )}
      </AccordionSection>
      <AccordionSection
        title="Agents"
        open={open.agents}
        onToggle={() => setOpen((s) => ({ ...s, agents: !s.agents }))}
      >
        {subagents.length === 0 ? (
          <Empty>No agents</Empty>
        ) : (
          subagents.map((a, i) => (
            <div key={`${a.name}-${i}`} className="flex items-center justify-between gap-2 px-5 py-1.5">
              <span className="min-w-0 flex-1 truncate text-[12px] text-koma-fg">{a.name}</span>
              <StatusBadge status={a.status} />
            </div>
          ))
        )}
      </AccordionSection>
    </div>
  )
}
