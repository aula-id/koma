import { useEffect, useState } from 'react'
import {
  Bot,
  Terminal,
  Check,
  CircleX,
  CircleSlash,
  X,
  FileText,
  Circle,
  CircleDot,
  CheckCircle2,
  Lock,
  ArrowDownToLine,
  type LucideIcon,
} from 'lucide-react'
import { AccordionSection } from '../AccordionSection'
import { Empty } from './helpers'
import { useKoma, visiblePlanTodos } from '../../store/koma'
import { BrailleSpinner } from '../BrailleSpinner'

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
  running: Check, // unused — StatusBadge renders BrailleSpinner for running
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
      {status === 'running' ? (
        <BrailleSpinner size={13} />
      ) : (
        <Icon size={13} strokeWidth={2} />
      )}
    </span>
  )
}

// Plan-todo status -> glyph + tone. pending = neutral outline, in_progress =
// accented (the live step), completed = dim + line-through (done items sink
// visually), cancelled = dim neutral (settled, no signal) — mirrors the
// Agents/Bash STATUS_ICON/TONE idiom above.
const PLAN_ICON: Record<string, LucideIcon> = {
  pending: Circle,
  in_progress: CircleDot,
  completed: CheckCircle2,
  cancelled: CircleSlash,
}

const PLAN_ICON_TONE: Record<string, string> = {
  pending: 'text-koma-fg opacity-45',
  in_progress: 'text-koma-accent',
  completed: 'text-koma-dim opacity-60',
  cancelled: 'text-koma-dim opacity-45',
}

const PLAN_TEXT_TONE: Record<string, string> = {
  pending: 'text-koma-fg',
  in_progress: 'text-koma-accent',
  completed: 'text-koma-dim opacity-60 line-through',
  cancelled: 'text-koma-dim opacity-45 line-through',
}

// Kill button for a running Agent/Bash row — mirrors the TUI's Ctrl+X kill.
// Only rendered while the job is running; emits the id-targeted kill GuiReq.
function KillBtn({ onClick }: { onClick: () => void }) {
  return (
    <button
      onClick={(e) => {
        e.stopPropagation()
        onClick()
      }}
      aria-label="Kill"
      title="Kill"
      className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-0 transition group-hover:opacity-60 hover:!text-koma-error hover:!opacity-100"
    >
      <X size={13} strokeWidth={2} />
    </button>
  )
}

// Background button for a running-and-blocking Agent row — mirrors the TUI's
// Ctrl+B-on-selection. Only rendered while the agent is running, not already
// detached, and still parking the main turn (`blocking`); flips it to detached
// without killing it (the agent keeps running, chat unblocks).
function BackgroundBtn({ onClick }: { onClick: () => void }) {
  return (
    <button
      onClick={(e) => {
        e.stopPropagation()
        onClick()
      }}
      aria-label="Background"
      title="Background (agent keeps running, chat unblocks)"
      className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-0 transition group-hover:opacity-60 hover:!text-koma-accent hover:!opacity-100"
    >
      <ArrowDownToLine size={13} strokeWidth={2} />
    </button>
  )
}

export function ExplorePanel() {
  const [open, setOpen] = useState({ plan: true, files: true, bash: true, agents: true })
  const subagents = useKoma((s) => s.session.subagents)
  const bash = useKoma((s) => s.session.bash)
  const files = useKoma((s) => s.session.fileChanges)
  const planTodos = useKoma((s) => s.session.planTodos)
  const mode = useKoma((s) => s.session.mode)
  const sdlcPhase = useKoma((s) => s.session.sdlcPhase)
  const sdlcGoal = useKoma((s) => s.session.sdlcGoal)
  const sdlcBranch = useKoma((s) => s.session.sdlcBranch)
  const sdlcOpen = useKoma((s) => s.session.sdlcOpen)
  const sdlcSealed = useKoma((s) => s.session.sdlcSealed)
  const focusPlanTick = useKoma((s) => s.ui.focusPlanTick)
  const req = useKoma((s) => s.req)
  const openDiffTab = useKoma((s) => s.openDiffTab)
  const openStreamTab = useKoma((s) => s.openStreamTab)

  const isPlan = mode === 'plan'
  const isSdlc = mode === 'sdlc'

  // Auto-expand PLAN the instant the session mode flips to 'plan' (also fires
  // on mount if the GUI (re)loads mid-plan). Never auto-collapses on leaving
  // Plan — the section's open/closed state otherwise persists exactly like
  // the other sections (user-driven only).
  useEffect(() => {
    if (isPlan || isSdlc) setOpen((s) => ({ ...s, plan: true }))
  }, [isPlan, isSdlc])

  // Cross-tree signal from the UsageFooter PLAN badge click: expand PLAN
  // (RootLayout's own effect on the same tick opens the sidebar/Explore view).
  useEffect(() => {
    if (focusPlanTick === 0) return
    setOpen((s) => ({ ...s, plan: true }))
  }, [focusPlanTick])

  const visiblePlan = visiblePlanTodos(planTodos)
  const planDone = visiblePlan.filter((t) => t.status === 'completed').length

  // SDLC rail data: only shown when mode=sdlc with SDLC fields present.
  const showSdlcRail = isSdlc && sdlcPhase != null
  const sdlcPhaseLabel = sdlcPhase ?? 'assess'
  const sdlcGoalLabel = sdlcGoal ?? ''
  const sdlcOpenCount = sdlcOpen ?? 0
  const sdlcSealedCount = sdlcSealed ?? 0
  const sdlcTotal = sdlcOpenCount + sdlcSealedCount

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <AccordionSection
        title={isSdlc && showSdlcRail
          ? (sdlcGoalLabel ? `SDLC · ${sdlcPhaseLabel} · ${sdlcSealedCount}/${sdlcTotal}` : `SDLC · ${sdlcPhaseLabel}`)
          : visiblePlan.length === 0 ? 'Plan' : `Plan · ${planDone}/${visiblePlan.length}`
        }
        open={open.plan}
        onToggle={() => setOpen((s) => ({ ...s, plan: !s.plan }))}
      >
        {isSdlc && showSdlcRail ? (
          // SDLC: phase/goal/branch/counts + graph task list (host planTodos projection).
          <div className="flex flex-col gap-1">
            <div className="flex min-h-[30px] items-center gap-2.5 px-3 py-1">
              <CircleDot size={12} className="flex-none text-koma-accent" />
              <span className="min-w-0 flex-1 truncate font-mono text-[12px] font-normal text-koma-accent">{sdlcPhaseLabel}</span>
            </div>
            {sdlcGoalLabel && (
              <div className="flex min-h-[30px] items-center gap-2.5 px-3 py-1">
                <FileText size={12} className="flex-none text-koma-fg opacity-45" />
                <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-koma-fg">{sdlcGoalLabel}</span>
              </div>
            )}
            {sdlcBranch && (
              <div className="flex min-h-[30px] items-center gap-2.5 px-3 py-1">
                <Lock size={12} className="flex-none text-koma-dim opacity-45" />
                <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-koma-dim opacity-60">{sdlcBranch}</span>
              </div>
            )}
            {sdlcTotal > 0 && (
              <div className="flex min-h-[30px] items-center gap-2.5 px-3 py-1">
                <Check size={12} className="flex-none text-koma-success" />
                <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-koma-fg">{sdlcSealedCount}/{sdlcTotal} sealed</span>
              </div>
            )}
            {planTodos.length === 0 ? (
              <Empty>No mission tasks yet</Empty>
            ) : (
              planTodos.map((t, i) => {
                const Icon = t.locked ? Lock : (PLAN_ICON[t.status] ?? Circle)
                const tone = t.locked ? 'text-koma-dim opacity-45' : (PLAN_ICON_TONE[t.status] ?? 'text-koma-fg opacity-45')
                const textTone = t.locked ? 'text-koma-dim opacity-45' : (PLAN_TEXT_TONE[t.status] ?? 'text-koma-fg')
                return (
                  <div key={i} className="flex min-h-[30px] items-center gap-2.5 px-3 py-1">
                    <Icon size={12} className={`flex-none ${tone}`} />
                    <span className={`min-w-0 flex-1 truncate font-mono text-[12px] font-normal ${textTone}`}>{t.content}</span>
                  </div>
                )
              })
            )}
          </div>
        ) : isPlan ? (
          // Plan checklist: only when mode=plan. Never leaks from other modes.
          planTodos.length === 0 ? (
            <Empty>No todos yet</Empty>
          ) : (
            planTodos.map((t, i) => {
              const Icon = t.locked ? Lock : (PLAN_ICON[t.status] ?? Circle)
              const tone = t.locked ? 'text-koma-dim opacity-45' : (PLAN_ICON_TONE[t.status] ?? 'text-koma-fg opacity-45')
              const textTone = t.locked ? 'text-koma-dim opacity-45' : (PLAN_TEXT_TONE[t.status] ?? 'text-koma-fg')
              return (
                <div key={i} className="flex min-h-[30px] items-center gap-2.5 px-3 py-1">
                  <Icon size={12} className={`flex-none ${tone}`} />
                  <span className={`min-w-0 flex-1 truncate font-mono text-[12px] font-normal ${textTone}`}>{t.content}</span>
                </div>
              )
            })
          )
        ) : (
          // Not in Plan or SDLC mode: show empty state (no stale rows leak).
          <Empty>No plan active</Empty>
        )}
      </AccordionSection>
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
                onClick={() => openDiffTab(f.path)}
                className="group flex min-h-[30px] cursor-pointer items-center gap-2.5 px-3 py-1 hover:bg-koma-hover"
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
          [...bash].reverse().map((b) => {
            const jobId = Number(String(b.id).replace(/^bash-/, ''))
            return (
              <div
                key={b.id}
                onClick={() => openStreamTab('bash', jobId, b.cmd)}
                title={`Stream output: ${b.cmd}`}
                className="group flex min-h-[30px] cursor-pointer items-center gap-2.5 px-3 py-1 hover:bg-koma-hover"
              >
                <Terminal size={13} className="flex-none text-koma-fg opacity-45" />
                <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-koma-fg">{b.cmd}</span>
                <StatusBadge status={b.status} />
                {b.status === 'running' && <KillBtn onClick={() => req({ r: 'KillBash', id: jobId })} />}
              </div>
            )
          })
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
              <div
                key={id ?? `${a.name}-${i}`}
                onClick={id != null ? () => openStreamTab('subagent', id, a.name) : undefined}
                title={id != null ? `Stream transcript: ${a.name}` : undefined}
                className={`group flex min-h-[30px] items-center gap-2.5 px-3 py-1 hover:bg-koma-hover ${id != null ? 'cursor-pointer' : ''}`}
              >
                <Bot size={13} className="flex-none text-koma-fg opacity-45" />
                <span className="min-w-0 flex-1 truncate text-[13px] text-koma-fg">{a.name}</span>
                {a.status === 'running' && a.detached && (
                  <span className="flex-none text-[10px] font-medium uppercase tracking-wide text-koma-dim opacity-60" title="Running in the background">
                    bg
                  </span>
                )}
                <StatusBadge status={a.status} />
                {a.status === 'running' && !a.detached && a.blocking && id != null && (
                  <BackgroundBtn onClick={() => req({ r: 'BackgroundSubagent', id })} />
                )}
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
