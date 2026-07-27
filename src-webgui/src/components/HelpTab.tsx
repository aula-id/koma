import { useRef, useState, type ReactNode } from 'react'
import { Info, Keyboard, Layers, PanelsTopLeft, Terminal } from 'lucide-react'

// Static, wire-free reference page for the GUI's own features — no requests,
// no live data, just documentation. Rendered as a tab over the main content
// column (see routes/index.tsx TabbedMain), mirroring the Settings tab's
// visual language exactly: a left nav rail + a single scrollable content pane,
// every colour a theme token (koma-* Tailwind classes / CSS vars), no
// hardcoded colours, no emoji. Every claim below is verified against the
// current component code — see the section comments for what NOT to claim
// (e.g. Esc does not interrupt a running turn; there's no keyboard shortcut
// for that yet).

type SectionId = 'composer' | 'sessions' | 'tabs' | 'keyboard' | 'tui'

const SECTIONS: { id: SectionId; label: string; icon: typeof Terminal }[] = [
  { id: 'composer', label: 'Composer', icon: Terminal },
  { id: 'sessions', label: 'Sessions', icon: Layers },
  { id: 'tabs', label: 'Tabs & panels', icon: PanelsTopLeft },
  { id: 'keyboard', label: 'Keyboard', icon: Keyboard },
  { id: 'tui', label: 'TUI-only', icon: Info },
]

export default function HelpTab() {
  const scrollRef = useRef<HTMLDivElement>(null)
  const refs = useRef<Partial<Record<SectionId, HTMLElement | null>>>({})
  const [active, setActive] = useState<SectionId>('composer')

  const goto = (id: SectionId) => {
    setActive(id)
    refs.current[id]?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  }

  // Scroll spy: highlight whichever section's header is nearest the top of the
  // pane (matches SettingsTab's own approach, generalised to N sections).
  const onScroll = () => {
    const pane = scrollRef.current
    if (!pane) return
    const paneTop = pane.getBoundingClientRect().top
    let current: SectionId = SECTIONS[0].id
    for (const { id } of SECTIONS) {
      const el = refs.current[id]
      if (!el) continue
      if (el.getBoundingClientRect().top - paneTop < 80) current = id
    }
    setActive(current)
  }

  return (
    <div className="flex h-full w-full min-w-0 bg-koma-bg text-koma-fg">
      <nav className="flex w-40 flex-none flex-col gap-0.5 border-r border-koma-border bg-koma-panel2 p-2">
        <div className="px-2 pb-1.5 pt-1 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-40">
          Help
        </div>
        {SECTIONS.map(({ id, label, icon: Icon }) => (
          <NavItem
            key={id}
            icon={<Icon size={15} />}
            label={label}
            active={active === id}
            onClick={() => goto(id)}
          />
        ))}
      </nav>

      <div ref={scrollRef} onScroll={onScroll} className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl px-8 py-6">
          <section ref={(el) => { refs.current.composer = el }}>
            <SectionHeader title="Composer" desc="What the message box does beyond plain text." />
            <InfoRow label="! shell command">
              A draft starting with <Code>!</Code> runs the rest of the line as a local shell command instead of
              sending a message — no model round-trip. Only fires while idle with no attachments staged; otherwise
              it's sent as a normal message.
            </InfoRow>
            <InfoRow label="File search">
              The magnifier icon opens a fuzzy workspace file search. Picking a result inserts it into the draft as
              an <Code>@name</Code> reference. Backspace right next to one deletes the whole reference in one
              keystroke, not character by character.
            </InfoRow>
            <InfoRow label="Up / Down — recall">
              With the caret on the first line, Up steps backward through your previously sent messages (oldest
              older); with the caret on the last line, Down steps forward and, once past the newest, restores
              whatever you'd been drafting. Editing the draft or sending resets the recall position.
            </InfoRow>
            <InfoRow label="Enter / Shift+Enter">
              Enter sends the draft. Shift+Enter inserts a newline instead.
            </InfoRow>
            <InfoRow label="Stopping a turn">
              There's no keyboard shortcut for this — use the stop button that appears in the composer while a turn
              is running.
            </InfoRow>
          </section>

          <section ref={(el) => { refs.current.sessions = el }} className="mt-12">
            <SectionHeader title="Sessions" desc="Switching, killing, deleting, and renaming sessions." />
            <InfoRow label="Change session">
              The "change session" pill in the titlebar opens a searchable list of cooking (live) sessions and past
              history. It stays visible even with nothing attached.
            </InfoRow>
            <InfoRow label="Kill vs. delete forever">
              On a cooking row, the trailing icon stops that session's daemon but leaves it on disk — it moves to
              History and can be reopened later. On a history row, the trailing icon deletes the session from disk
              permanently — there's no undo. Killing the currently-attached session drops you back to the start
              screen once the daemon is confirmed stopped.
            </InfoRow>
            <InfoRow label="Multi-select">
              In the start screen Recent list and the change-session hub, a plain click selects and highlights a
              row (it does not open it). Ctrl/⌘-click toggles; Shift-click selects a range. Double-click or Enter
              opens/resumes. With one or more rows selected, the bulk bar offers Kill (live) and Delete forever
              (history), each with a yes/no confirm. Esc clears the selection first.
            </InfoRow>
            <InfoRow label="+ New session">
              The primary button opens a folder picker for a new session and leaves whatever's currently cooking
              running in the background. The chevron next to it offers "New session + close current," which stops
              the current session's daemon first (same as a kill — it moves to History, it isn't deleted) before
              opening the picker.
            </InfoRow>
            <InfoRow label="Rename">
              The "rename" pill in the titlebar renames the current session; only shown while a session is attached.
            </InfoRow>
            <InfoRow label="Compact">
              The "compact" button in the titlebar compacts the conversation context on demand. Disabled while a
              turn is running.
            </InfoRow>
          </section>

          <section ref={(el) => { refs.current.tabs = el }} className="mt-12">
            <SectionHeader title="Tabs & panels" desc="What each tab and sidebar panel is for." />
            <InfoRow label="Chat">The permanent first tab — it can't be closed.</InfoRow>
            <InfoRow label="Settings">
              Appearance (theme picker) and session preferences (name, working directories, short-send, sliding
              cache, bash shorts, internet mode). Opened from the gear at the bottom of the activity bar; closeable.
            </InfoRow>
            <InfoRow label="Diff tabs">
              Click a row under the Explorer's "File changed" section to open a side-by-side diff. If the workspace
              isn't a git repo, a "session baseline" badge marks that the original side came from the session's own
              first-touch snapshot rather than git.
            </InfoRow>
            <InfoRow label="Sub-agent / bash stream tabs">
              Read-only. Opened from the Explorer's Agents/Bash rows, they live-update while their target is
              running and stay open (closeable) afterward.
            </InfoRow>
            <InfoRow label="Explorer sidebar">
              Plan (the todo checklist while in Plan mode), File changed, Bash (running/finished jobs, with a kill
              button while running), and Agents (sub-agents, with background/kill buttons while running).
            </InfoRow>
            <InfoRow label="MCP panel">Add, edit, enable, or remove MCP servers.</InfoRow>
            <InfoRow label="Connector panel">Providers, OAuth, and the model catalogue.</InfoRow>
            <InfoRow label="Usage panel">
              A last-7-days cost/token preview with an all-sessions/this-session scope toggle (the session scope is
              only available while a session is attached) and a top-models-by-cost list.
            </InfoRow>
          </section>

          <section ref={(el) => { refs.current.keyboard = el }} className="mt-12">
            <SectionHeader title="Keyboard" desc="Shortcuts that work anywhere in the app, not just one field." />
            <InfoRow label="Ctrl+B (Cmd+B on mac)">
              Backgrounds every eligible running sub-agent at once.
            </InfoRow>
            <InfoRow label="Ctrl+R (Cmd+R on mac)">
              Resends your last message. Only acts while idle (it still swallows the key either way, so the
              browser's own reload shortcut never fires).
            </InfoRow>
            <InfoRow label="Escape">
              Closes whatever dropdown, palette, or overlay is currently open (change-session, rename, new-session
              menu, model/mode/effort pickers) — or, on an armed kill/delete row, cancels the arm first.
            </InfoRow>
            <p className="mt-3 text-[11.5px] leading-relaxed text-koma-fg opacity-45">
              Both Ctrl+B and Ctrl+R are exempted while focus is inside a diff tab's editor, which keeps its own
              bindings.
            </p>
          </section>

          <section ref={(el) => { refs.current.tui = el }} className="mt-12">
            <SectionHeader title="TUI-only" desc="A few things live only in the terminal client, by design." />
            <p className="text-[12.5px] leading-relaxed text-koma-fg opacity-80">
              The security toolkit and its playbooks, and power-user slash commands like a raw session/context dump,
              are terminal-only. This desktop shell focuses on the everyday flow — sessions, chat, diffs, and the
              panels above.
            </p>
          </section>
        </div>
      </div>
    </div>
  )
}

function Code({ children }: { children: ReactNode }) {
  return (
    <code className="rounded bg-koma-panel2 px-1 py-0.5 font-mono text-[11px] text-koma-fg">{children}</code>
  )
}

function NavItem({
  icon,
  label,
  active,
  onClick,
}: {
  icon: ReactNode
  label: string
  active: boolean
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`flex items-center gap-2 rounded px-2 py-1.5 text-left text-[12.5px] transition-colors ${
        active
          ? 'bg-koma-hover text-koma-fg'
          : 'text-koma-fg opacity-60 hover:bg-koma-hover hover:opacity-90'
      }`}
    >
      <span className={`flex-none ${active ? 'text-koma-accent' : 'opacity-70'}`}>{icon}</span>
      <span className="truncate">{label}</span>
    </button>
  )
}

function SectionHeader({ title, desc }: { title: string; desc: string }) {
  return (
    <div className="mb-4 border-b border-koma-border pb-2">
      <h2 className="text-[15px] font-semibold text-koma-fg">{title}</h2>
      <p className="mt-0.5 text-[12px] text-koma-fg opacity-45">{desc}</p>
    </div>
  )
}

function InfoRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="border-b border-koma-border py-3.5">
      <div className="text-[13px] text-koma-fg">{label}</div>
      <div className="mt-1 text-[11.5px] leading-relaxed text-koma-fg opacity-60">{children}</div>
    </div>
  )
}
