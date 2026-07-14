import { useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import {
  Bot,
  Terminal,
  Loader2,
  Check,
  CircleX,
  CircleSlash,
  Brain,
  ChevronDown,
  ChevronRight,
  type LucideIcon,
} from 'lucide-react'
import { useKoma, type Tab, type SubAgentEntry, type BashJobEntry } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'

// Read-only STREAM tab: live-streams ONE sub-agent's transcript or ONE bash job's
// output into a dedicated scrollable view (the non-key equivalent of the TUI's
// full-screen sub-agent viewer, generalised to bash). Lazy-loaded (like DiffTab), so
// nothing here touches the main bundle until the first stream tab is opened. Content is
// read LIVE off the store (the host folds the viewed target's transcript/output tail into
// each Snapshot for exactly the sub-agent/bash job the active stream view names), so the
// view updates as content streams in.

type StreamTabModel = Extract<Tab, { kind: 'subagent' | 'bash' }>

// status -> icon/tone, mirroring the ExplorePanel row grammar (running spins).
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

function isTerminal(status: string): boolean {
  return status === 'done' || status === 'killed' || status === 'error'
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

function Centered({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-full w-full items-center justify-center px-6 text-center text-[12px] text-koma-dim">
      {children}
    </div>
  )
}

function Loading() {
  return (
    <div className="flex h-full w-full items-center justify-center text-koma-dim">
      <BrailleSpinner size={18} className="opacity-70" />
    </div>
  )
}

// The header row (icon + title + live status), with an optional dim subtitle line for a
// sub-agent's compact task summary.
function StreamHeader({
  Icon,
  title,
  status,
  subtitle,
}: {
  Icon: LucideIcon
  title: string
  status?: string
  subtitle?: string
}) {
  return (
    <div className="flex-none border-b border-koma-border bg-koma-panel2 px-3 py-1.5">
      <div className="flex items-center gap-2">
        <Icon size={14} className="flex-none text-koma-fg opacity-70" />
        <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-koma-fg">{title}</span>
        {status && <StatusBadge status={status} />}
        {status && <span className="flex-none text-[11px] text-koma-dim">{status}</span>}
      </div>
      {subtitle && <div className="mt-0.5 truncate pl-6 text-[11px] text-koma-dim">{subtitle}</div>}
    </div>
  )
}

// Collapsible dim thinking block (mirrors ChatView's ReasoningBlock idiom).
function ThinkingBlock({ text }: { text: string }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="mb-2">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex items-center gap-1 text-[11px] text-koma-dim opacity-70 transition-opacity hover:opacity-100"
      >
        <Brain size={11} className="flex-none" />
        <span>thinking</span>
        {open ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
      </button>
      {open && (
        <div className="mt-1 whitespace-pre-wrap border-l-2 border-koma-dim pl-2 text-[12px] italic text-koma-dim">
          {text}
        </div>
      )}
    </div>
  )
}

// Scroll-anchored body: auto-stick to the bottom as content grows, RELEASE when the user
// scrolls up to read back, RE-STICK at the bottom — the same stickRef pattern ChatView
// uses. `deps` re-runs the bottom-pin when content changes.
function useBottomStick(deps: unknown[]) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const stickRef = useRef(true)
  const onScroll = () => {
    const el = scrollRef.current
    if (!el) return
    stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40
  }
  useLayoutEffect(() => {
    if (!stickRef.current) return
    const el = scrollRef.current
    if (el) el.scrollTop = el.scrollHeight
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps)
  return { scrollRef, onScroll }
}

const BODY_CLASS =
  'h-full overflow-y-auto px-3 py-2 font-mono text-[12px] leading-relaxed text-koma-fg [scrollbar-width:thin]'

function SubAgentStream({ entry, title }: { entry?: SubAgentEntry; title: string }) {
  const transcript = entry?.transcript
  const liveText = entry?.liveText
  const thinking = entry?.thinking
  const { scrollRef, onScroll } = useBottomStick([transcript, liveText, thinking])
  const status = entry?.status

  let body: ReactNode
  if (!entry) {
    // The agent dropped out of the foreground session's list (rare — finished agents are
    // retained; a session switch closes the tab instead of reaching here).
    body = <Centered>This sub-agent is no longer available.</Centered>
  } else if (transcript === undefined) {
    // Viewed, but the host hasn't folded in the transcript yet (≤1 fold tick).
    body = <Loading />
  } else if (transcript.length === 0 && !liveText) {
    body =
      status && isTerminal(status) ? (
        <Centered>transcript not persisted for restored records</Centered>
      ) : (
        <Centered>waiting for output…</Centered>
      )
  } else {
    body = (
      <div ref={scrollRef} onScroll={onScroll} className={BODY_CLASS}>
        {thinking && <ThinkingBlock text={thinking} />}
        {transcript.map((line, i) => (
          <div key={i} className="whitespace-pre-wrap break-words">
            {line || ' '}
          </div>
        ))}
        {liveText && (
          <div className="mt-1 whitespace-pre-wrap break-words italic text-koma-dim">{liveText}</div>
        )}
      </div>
    )
  }

  return (
    <div className="flex h-full w-full flex-col">
      <StreamHeader Icon={Bot} title={entry?.name ?? title} status={status} subtitle={entry?.summary} />
      <div className="min-h-0 flex-1">{body}</div>
    </div>
  )
}

function BashStream({ entry, title }: { entry?: BashJobEntry; title: string }) {
  const outputTail = entry?.outputTail
  const { scrollRef, onScroll } = useBottomStick([outputTail])
  const status = entry?.status

  let body: ReactNode
  if (!entry) {
    body = <Centered>This bash job is no longer available.</Centered>
  } else if (outputTail === undefined) {
    body = <Loading />
  } else if (outputTail === '') {
    body =
      status && isTerminal(status) ? (
        <Centered>output not persisted for restored records</Centered>
      ) : (
        <Centered>waiting for output…</Centered>
      )
  } else {
    body = (
      <div ref={scrollRef} onScroll={onScroll} className={BODY_CLASS}>
        <pre className="whitespace-pre-wrap break-words">{outputTail}</pre>
      </div>
    )
  }

  return (
    <div className="flex h-full w-full flex-col">
      <StreamHeader Icon={Terminal} title={entry?.cmd ?? title} status={status} />
      <div className="min-h-0 flex-1">{body}</div>
    </div>
  )
}

export default function StreamTab({ tab }: { tab: StreamTabModel }) {
  const subagents = useKoma((s) => s.session.subagents)
  const bash = useKoma((s) => s.session.bash)

  if (tab.kind === 'subagent') {
    const entry = subagents.find((a) => a.id === tab.agentId)
    return <SubAgentStream entry={entry} title={tab.title} />
  }
  // Bash rows carry a string id (`bash-<n>`); match the tab's numeric jobId.
  const entry = bash.find((b) => Number(String(b.id).replace(/^bash-/, '')) === tab.jobId)
  return <BashStream entry={entry} title={tab.title} />
}
