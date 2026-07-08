import { memo, useLayoutEffect, useRef, useState, type ComponentType } from 'react'
import {
  Archive,
  Brain,
  Check,
  ChevronDown,
  ChevronRight,
  Circle,
  Cog,
  FileText,
  Files,
  Folder,
  GitBranch,
  Globe,
  Image as ImageIcon,
  Images,
  MessageSquare,
  Pencil,
  Plug,
  Search,
  Shield,
  Terminal,
} from 'lucide-react'
import { useKoma, type AttachmentEntry, type ChatMessage, type ToolCallView } from '../store/koma'
import { MessageBody } from './MessageBody'
import { Composer } from './Composer'
import { ApprovalOverlay } from './ApprovalOverlay'
import { fallbackSignature, truncateChars } from '../lib/toolSignature'

// Native chat view — a 1:1 clone of the TUI `view::chat` render grammar
// (src-agent/src/view/chat/*), with every box-drawing/unicode glyph swapped
// for a lucide-react icon and every color routed through the --koma-* palette
// roles (never raw hex). Reads the koma store (mirror of the host's
// authoritative push envelopes) and NEVER accumulates locally: history comes
// straight from session.messages, and the live in-flight reply is a single
// bubble driven by session.stream/session.reasoning. There is NO TUI header —
// the GUI has its own titlebar chrome.

type IconType = ComponentType<{ size?: number; className?: string }>

// ---- Tool metadata: mirrors transcript.rs `tool_box_label` (the whitelist of
// output-producing tools that get a boxed result) + a per-family lucide icon.
// A null result means the tool falls back to the terse one-liner path.
function toolBoxMeta(name: string): { label: string; Icon: IconType } | null {
  if (name === 'bash') return { label: 'bash', Icon: Terminal }
  if (name === 'read') return { label: 'read', Icon: FileText }
  if (name === 'grep') return { label: 'grep', Icon: Search }
  if (name === 'glob') return { label: 'glob', Icon: Files }
  if (name === 'dir_list') return { label: 'dir', Icon: Folder }
  if (name.startsWith('git_')) return { label: 'git', Icon: GitBranch }
  if (name.startsWith('web_')) return { label: 'web', Icon: Globe }
  if (name === 'recall') return { label: 'memory', Icon: Brain }
  if (name.startsWith('mcp__')) return { label: 'mcp', Icon: Plug }
  if (name.startsWith('sec_')) return { label: 'sec', Icon: Shield }
  return null
}

// plan_ready digest: the composed checklist + plan Markdown the daemon rewrites
// into the tool call's `highlights` arg (transcript.rs `render_tool_lines` reads
// the same field). Parsed out so it renders as a real Markdown block instead of
// the terse `plan_ready(…)` signature. Returns null on any parse miss → the
// caller falls back to the generic tool-call line.
function planDigest(args: string): string | null {
  try {
    const parsed = JSON.parse(args)
    const h = parsed?.highlights
    return typeof h === 'string' && h.trim() !== '' ? h : null
  } catch {
    return null
  }
}

// ---- Tool RESULT: the dotted box (helpers.rs `render_tool_box`). Rounded,
// light-dashed edges → a CSS `border border-dashed` card; the `╭┄ label ┄╮`
// chip → a family icon + accent label; `┆` side glyphs → the dashed border.
// Max 5 source lines, each TRUNCATED (never wrapped); a `…` overflow row when
// there are more. Body text is dim + italic.
const ToolOutputBox = memo(function ToolOutputBox({
  label,
  Icon,
  output,
}: {
  label: string
  Icon: IconType
  output: string
}) {
  const lines = output.replace(/\s+$/, '').split('\n')
  const shown = lines.slice(0, 5)
  const overflow = lines.length > 5
  return (
    <div className="mt-1 ml-4 overflow-hidden rounded-md border border-dashed border-koma-dim/60">
      <div className="flex items-center gap-1 px-2 pt-1 text-[11px]">
        <Icon size={11} className="flex-none text-koma-accent" />
        <span className="text-koma-accent">{label}</span>
      </div>
      <div className="px-2 pb-1.5 pt-0.5 font-mono text-[11.5px] italic text-koma-dim">
        {shown.map((ln, i) => (
          <div key={i} className="truncate">
            {ln || ' '}
          </div>
        ))}
        {overflow && <div>…</div>}
      </div>
    </div>
  )
})

// Terse fallback for non-boxed tools (transcript.rs terse path): first
// non-blank line only, truncated to 80, dim, under a small indent.
function TerseResult({ output }: { output: string }) {
  const first = output.split('\n').find((l) => l.trim() !== '') ?? ''
  if (first.trim() === '') return null
  return (
    <div className="mt-0.5 ml-4 truncate font-mono text-[11.5px] text-koma-dim">
      {truncateChars(first.trim(), 80)}
    </div>
  )
}

// ---- One tool call + its inline result (transcript.rs `render_tool_lines`).
// Status glyph resolves live: in-flight `⚙` (Cog, spinning) in dim →
// completed `✓` (Check) in accent. Result is glued directly under its own call.
const ToolCallRow = memo(function ToolCallRow({ call }: { call: ToolCallView }) {
  const done = call.status === 'done'
  const req = useKoma((s) => s.req)
  // The session is PARKED on a plan decision when the approval gate is set AND
  // the paused call is `plan_ready`. Only the currently-parked plan_ready row is
  // still pending (`!done`); historical plan_ready rows in scrollback already
  // got answered → they have a result → done. So gate the controls on both, so
  // they show on exactly the one row awaiting a decision.
  const awaitingApproval = useKoma((s) => s.session.awaitingApproval)
  const pendingIsPlan = useKoma((s) => s.session.pendingCall?.name === 'plan_ready')

  // plan_ready: a "plan ready" header + the FULL Markdown digest (the rewritten
  // `highlights` arg) rendered behind the dim quote rail — mirrors the TUI's
  // render_tool_lines special case, NOT the terse one-line signature. Falls
  // through to the generic row if the digest can't be parsed out.
  if (call.name === 'plan_ready') {
    const digest = planDigest(call.args)
    if (digest != null) {
      const showControls = !done && awaitingApproval && pendingIsPlan
      const decide = (decision: 'approve' | 'compact' | 'deny') =>
        req({ r: 'PlanDecision', decision })
      return (
        <div>
          <div className="flex items-center gap-1.5 text-[12.5px] text-koma-dim">
            {done ? (
              <Check size={12} className="flex-none text-koma-accent" />
            ) : (
              <Cog size={12} className="flex-none animate-spin text-koma-dim" />
            )}
            <span>plan ready</span>
          </div>
          <div className="mt-1 border-l-2 border-koma-dim pl-3">
            <MessageBody text={digest} />
          </div>
          {showControls && (
            <div className="mt-2 flex flex-wrap items-center gap-2 pl-3">
              <button
                onClick={() => decide('approve')}
                className="flex items-center gap-1.5 rounded-md border border-koma-accent bg-koma-accent/15 px-2.5 py-1 text-[12px] text-koma-accent transition-colors hover:bg-koma-accent/25"
              >
                <Check size={13} className="flex-none" />
                Approve
              </button>
              <button
                onClick={() => decide('compact')}
                className="flex items-center gap-1.5 rounded-md border border-koma-border bg-koma-panel px-2.5 py-1 text-[12px] text-koma-fg opacity-85 transition-colors hover:bg-koma-hover hover:opacity-100"
              >
                <Archive size={13} className="flex-none" />
                Approve &amp; compact
              </button>
              <button
                onClick={() => decide('deny')}
                className="flex items-center gap-1.5 rounded-md border border-koma-border bg-koma-panel px-2.5 py-1 text-[12px] text-koma-fg opacity-85 transition-colors hover:bg-koma-hover hover:opacity-100"
              >
                <MessageSquare size={13} className="flex-none" />
                Chat more
              </button>
            </div>
          )}
        </div>
      )
    }
  }

  const meta = toolBoxMeta(call.name)
  const signature = call.signature ?? fallbackSignature(call.name, call.args)
  const hasOutput = call.output != null && call.output.trim() !== ''
  return (
    <div>
      <div className="flex items-center gap-1.5 text-[12.5px] text-koma-dim">
        {done ? (
          <Check size={12} className="flex-none text-koma-accent" />
        ) : (
          <Cog size={12} className="flex-none animate-spin text-koma-dim" />
        )}
        <span className="truncate">{signature}</span>
      </div>
      {hasOutput &&
        (meta ? (
          <ToolOutputBox label={meta.label} Icon={meta.Icon} output={call.output as string} />
        ) : (
          <TerseResult output={call.output as string} />
        ))}
    </div>
  )
})

// ---- Reasoning channel (transcript.rs `▏ ` THINK_BAR): dim + italic, behind a
// thin left border (the `▏` glyph) fronted by a Brain icon. Collapsible; opens
// by default while streaming so live thinking is visible, collapses once done.
function ReasoningBlock({ text, defaultOpen }: { text: string; defaultOpen: boolean }) {
  const [open, setOpen] = useState(defaultOpen)
  return (
    <div className="mb-1.5">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex items-center gap-1 text-[11px] text-koma-dim opacity-70 transition-opacity hover:opacity-100"
      >
        <Brain size={11} className="flex-none" />
        <span>reasoning</span>
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

// ---- Image attachment card (transcript.rs `render_attachment_card`): the warn-
// coloured list under a user message that carries `[Image #N]` attachments. The
// TUI's `images` folder-tree (├─/└─ connectors) maps to an Images-icon header +
// one ImageIcon row per attachment, all in the warn role — koma can't guarantee
// the model actually read the image, so it's always a warn cue.
function AttachmentCard({ attachments }: { attachments: AttachmentEntry[] }) {
  return (
    <div className="mt-1.5 rounded-md border border-dashed border-koma-warn/50 px-2 py-1.5 text-[11.5px] text-koma-warn">
      <div className="flex items-center gap-1">
        <Images size={12} className="flex-none" />
        <span>images</span>
      </div>
      <div className="mt-0.5 space-y-0.5 pl-1">
        {attachments.map((a) => (
          <div key={a.markerN} className="flex items-center gap-1 opacity-90">
            <ImageIcon size={11} className="flex-none" />
            <span className="truncate">
              [Image #{a.markerN}] {a.name}
            </span>
          </div>
        ))}
      </div>
    </div>
  )
}

// ---- USER message: the full-width BAND (transcript.rs `render_user_message`).
// `▌` left rail (fg=accent) → a solid accent left bar; text = accent on the
// panel-tinted band; runs edge-to-edge. When the message carries image
// attachments, the warn card hangs below it (mirrors the TUI: content first,
// then the card).
function UserMessage({
  content,
  attachments,
  onEdit,
}: {
  content: string
  attachments?: AttachmentEntry[]
  // Hover-edit → rewind: drops everything after this message and refills the
  // composer with its text (mirrors the TUI's double-Esc MessageRewind). Absent
  // → no pencil (e.g. the live view, where the index isn't a committed message).
  onEdit?: () => void
}) {
  return (
    <div className="group relative">
      <div className="flex overflow-hidden bg-koma-band">
        <div className="w-[3px] flex-none bg-koma-accent" />
        <div className="min-w-0 flex-1 whitespace-pre-wrap px-3 py-2 text-[13px] text-koma-accent">
          {content}
        </div>
      </div>
      {onEdit && (
        <button
          onClick={onEdit}
          aria-label="Edit & rewind to here"
          title="Edit & rewind to here"
          className="absolute right-1.5 top-1.5 flex h-6 w-6 items-center justify-center rounded-md bg-koma-panel text-koma-fg opacity-0 shadow-sm transition-opacity hover:!opacity-100 hover:bg-koma-hover group-hover:opacity-70"
        >
          <Pencil size={12} />
        </button>
      )}
      {attachments && attachments.length > 0 && <AttachmentCard attachments={attachments} />}
    </div>
  )
}

// ---- `!` user-shell entry (transcript.rs `render_shell_block`): a `$ <cmd>`
// header in accent over the captured output in dim. The host strips SHELL_MARK
// and tags kind='shell'; `content` is `$ <cmd>\n<output…>`. A header-less body
// degrades gracefully (the whole first line becomes the command).
function ShellMessage({ content }: { content: string }) {
  const lines = content.split('\n')
  const header = lines[0] ?? '$'
  const cmd = header.startsWith('$ ') ? header.slice(2) : header
  const output = lines.slice(1).join('\n')
  return (
    <div className="font-mono text-[12.5px]">
      <div className="flex items-start gap-1.5">
        <span className="flex-none select-none text-koma-accent">$</span>
        <span className="min-w-0 flex-1 whitespace-pre-wrap break-words text-koma-accent">{cmd}</span>
      </div>
      {output.trim() !== '' && (
        <div className="mt-0.5 whitespace-pre-wrap break-words pl-4 text-koma-dim">{output}</div>
      )}
    </div>
  )
}

// ---- bg-bash completion nudge (transcript.rs `render_bash_nudge_block`): ONE
// compact dim line — a success Check (the TUI's green `✓`, mapped to a lucide
// icon, never a literal char) + the summary (line 1 of the body). The remaining
// lines are model-only context and are NOT shown.
function BashNudgeMessage({ content }: { content: string }) {
  const summary = content.split('\n')[0] ?? ''
  if (summary.trim() === '') return null
  return (
    <div className="flex items-center gap-1.5 pl-1 text-[12px] text-koma-dim">
      <Check size={12} className="flex-none text-koma-success" />
      <span className="truncate">{summary}</span>
    </div>
  )
}

// ---- ASSISTANT message: `●` bullet (transcript.rs `render_message_block`) →
// a filled Circle in fg; reasoning above, markdown body (streaming-safe), then
// tool calls with inline results. The body/tools sit in the column offset by
// the bullet, matching the TUI's 2-space hang under `●`.
const AssistantMessage = memo(function AssistantMessage({
  content,
  reasoning,
  toolCalls,
  streaming,
}: {
  content: string
  reasoning: string | null
  toolCalls?: ToolCallView[]
  streaming: boolean
}) {
  const hasBody = content.trim() !== ''
  const hasReasoning = reasoning != null && reasoning.trim() !== ''
  const hasTools = toolCalls != null && toolCalls.length > 0
  return (
    <div className="flex gap-2">
      <Circle size={9} className="mt-[5px] flex-none fill-koma-fg text-koma-fg" />
      <div className="min-w-0 flex-1">
        {hasReasoning && <ReasoningBlock text={reasoning as string} defaultOpen={streaming} />}
        {hasBody && <MessageBody text={content} streaming={streaming} />}
        {hasTools && (
          <div className="mt-1 space-y-1">
            {(toolCalls as ToolCallView[]).map((c) => (
              <ToolCallRow key={c.id} call={c} />
            ))}
          </div>
        )}
      </div>
    </div>
  )
})

// Dispatch a committed message to its role framing. A user message may carry a
// special `kind` (host-detected + sentinel-stripped): shell / bash-nudge render
// distinctly, everything else is the normal accent band (+ attachment card).
function Message({ m, index }: { m: ChatMessage; index: number }) {
  const req = useKoma((s) => s.req)
  const refillComposer = useKoma((s) => s.refillComposer)
  if (m.role === 'user') {
    if (m.kind === 'shell') return <ShellMessage content={m.content} />
    if (m.kind === 'bashNudge') return <BashNudgeMessage content={m.content} />
    // Rewind TO this message: the daemon drops everything after `index` and
    // aborts any in-flight turn; we optimistically drop the text back into the
    // composer for editing + resend (the host also refills its own input, but
    // the GUI composer is local-state, so we mirror it here).
    const onEdit = () => {
      req({ r: 'RewindTo', index })
      refillComposer(m.content)
    }
    return <UserMessage content={m.content} attachments={m.attachments} onEdit={onEdit} />
  }
  return (
    <AssistantMessage
      content={m.content}
      reasoning={m.reasoning}
      toolCalls={m.toolCalls}
      streaming={false}
    />
  )
}

export function ChatView() {
  const messages = useKoma((s) => s.session.messages)
  const stream = useKoma((s) => s.session.stream)
  const reasoning = useKoma((s) => s.session.reasoning)
  const working = useKoma((s) => s.session.working)

  // Live in-flight bubble: shown once tokens or live reasoning arrive. Keyed at
  // its FUTURE index (messages.length) so that when the Snapshot commit lands
  // and the message joins the array, React reuses the same DOM node instead of
  // remounting it.
  const showLive = stream.length > 0 || (working && reasoning.trim() !== '')

  // Scroll-anchored to BOTTOM: auto-stick to the newest content as the
  // transcript / live stream grows, but RELEASE the moment the user scrolls up
  // to read back, and RE-STICK once they return to the bottom. `stickRef` is a
  // ref (not state) so the scroll handler never triggers a re-render, and the
  // pin runs in a layout effect (before paint) so streaming never flickers.
  const scrollRef = useRef<HTMLDivElement>(null)
  const stickRef = useRef(true)

  const onScroll = () => {
    const el = scrollRef.current
    if (!el) return
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight
    stickRef.current = distanceFromBottom < 40
  }

  useLayoutEffect(() => {
    if (!stickRef.current) return
    const el = scrollRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [messages, stream, reasoning, showLive])

  // Scroll-on-send: the composer bumps `scrollTick` on every submit. FORCE
  // re-engage the bottom-stick (even if the user had scrolled up to read back)
  // and snap down now, so a send while scrolled up never lands off-screen.
  const scrollTick = useKoma((s) => s.ui.scrollTick)
  useLayoutEffect(() => {
    stickRef.current = true
    const el = scrollRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [scrollTick])

  return (
    <div className="term-shell flex flex-col">
      <div
        ref={scrollRef}
        onScroll={onScroll}
        className="flex-1 space-y-4 overflow-y-auto px-2 py-4"
      >
        {messages.map((m, i) => (
          <Message key={i} m={m} index={i} />
        ))}
        {showLive && (
          <AssistantMessage
            key={messages.length}
            content={stream}
            reasoning={reasoning || null}
            streaming
          />
        )}
      </div>
      <ApprovalOverlay />
      <Composer />
    </div>
  )
}
