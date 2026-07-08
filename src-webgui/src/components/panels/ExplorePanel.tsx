import { useState } from 'react'
import { Bot, Terminal, Loader2, Check, CircleX, CircleSlash, X, FileText, type LucideIcon } from 'lucide-react'
import { AccordionSection } from '../AccordionSection'
import { Empty } from './helpers'
import { useKoma } from '../../store/koma'

// File-change status -> single-letter git-style badge + tone. added = new (good),
// modified = touched (accent), deleted = removed (error/red).
const FILE_STATUS: Record<string, { letter: string; tone: string }> = {
  added: { letter: 'A', tone: 'text-koma-success' },
  modified: { letter: 'M', tone: 'text-koma-accent' },
  deleted: { letter: 'D', tone: 'text-koma-error' },
}

// Show just the basename in the main label; the full path rides the tooltip.
function baseName(path: string): string {
  const parts = path.split('/')
  return parts[parts.length - 1] || path
}

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

// Kill button for a running Agent/Bash row — mirrors the TUI's Ctrl+X kill.
// Only rendered while the job is running; emits the id-targeted kill GuiReq.
function KillBtn({ onClick }: { onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      aria-label="Kill"
      title="Kill"
      className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-0 transition group-hover:opacity-60 hover:!text-koma-error hover:!opacity-100"
    >
      <X size={13} strokeWidth={2} />
    </button>
  )
}

export function ExplorePanel() {
  const [open, setOpen] = useState({ files: true, bash: true, agents: true })
  const subagents = useKoma((s) => s.session.subagents)
  const bash = useKoma((s) => s.session.bash)
  const files = useKoma((s) => s.session.fileChanges)
  const req = useKoma((s) => s.req)

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <AccordionSection
        title={files.length === 0 ? 'File changed' : `File changed · ${files.length}`}
        open={open.files}
        onToggle={() => setOpen((s) => ({ ...s, files: !s.files }))}
      >
        {files.length === 0 ? (
          <Empty>No changes</Empty>
        ) : (
          files.map((f) => {
            const meta = FILE_STATUS[f.status] ?? { letter: '?', tone: 'text-koma-dim' }
            return (
              <div
                key={f.path}
                title={`${f.status}: ${f.path}`}
                className="group flex min-h-[30px] items-center gap-2.5 px-3 py-1 hover:bg-koma-hover"
              >
                <FileText size={13} className="flex-none text-koma-fg opacity-45" />
                <span className={`min-w-0 flex-1 truncate font-mono text-[12px] text-koma-fg ${f.status === 'deleted' ? 'line-through opacity-60' : ''}`}>
                  {baseName(f.path)}
                </span>
                <span className={`flex-none font-mono text-[11px] font-semibold ${meta.tone}`}>{meta.letter}</span>
              </div>
            )
          })
        )}
      </AccordionSection>
      <AccordionSection
        title={`Bash · ${bash.length}`}
        open={open.bash}
        onToggle={() => setOpen((s) => ({ ...s, bash: !s.bash }))}
      >
        {bash.length === 0 ? (
          <Empty>No bash sessions</Empty>
        ) : (
          [...bash].reverse().map((b) => (
            <div key={b.id} className="group flex min-h-[30px] items-center gap-2.5 px-3 py-1 hover:bg-koma-hover">
              <Terminal size={13} className="flex-none text-koma-fg opacity-45" />
              <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-koma-fg">{b.cmd}</span>
              <StatusBadge status={b.status} />
              {b.status === 'running' && (
                <KillBtn onClick={() => req({ r: 'KillBash', id: Number(String(b.id).replace(/^bash-/, '')) })} />
              )}
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
          [...subagents].reverse().map((a, i) => {
            const id = a.id
            return (
              <div key={id ?? `${a.name}-${i}`} className="group flex min-h-[30px] items-center gap-2.5 px-3 py-1 hover:bg-koma-hover">
                <Bot size={13} className="flex-none text-koma-fg opacity-45" />
                <span className="min-w-0 flex-1 truncate text-[13px] text-koma-fg">{a.name}</span>
                <StatusBadge status={a.status} />
                {a.status === 'running' && id != null && (
                  <KillBtn onClick={() => req({ r: 'KillSubagent', id })} />
                )}
              </div>
            )
          })
        )}
      </AccordionSection>
    </div>
  )
}
