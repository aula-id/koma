import { useEffect, useRef, useState, type ReactNode } from 'react'
import {
  Check,
  Copy,
  ExternalLink,
  Eye,
  KeyRound,
  Palette as PaletteIcon,
  PanelLeft,
  Plus,
  SlidersHorizontal,
  Code2,
  Trash2,
  UserCircle,
  X,
  Download,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import { useKoma, resolveActivityBarOrder, type PaletteInfo } from '../store/koma'
import { ACTIVITY_BAR_ITEMS } from './ActivityBar'
import { Field, Segmented, TextInput, Toggle } from './panels/form'
import { BrailleSpinner } from './BrailleSpinner'

// VSCode-style Settings page, rendered as a tab over the main content column
// (see routes/index.tsx TabbedMain). A left nav rail scrolls the content pane to
// its sections; Appearance is a movie-strip palette grid, Session is a labelled
// list of the session prefs, SSH Keys is the host key-vault CRUD (wave 4a — a
// GUI-only, manual, user-owned vault, separate from the model's own git
// credential machinery). Every colour is a theme token (var(--koma-*) via the
// koma-* Tailwind classes) so it tracks the live palette.

type SectionId = 'account' | 'appearance' | 'session' | 'activityBar' | 'lsp' | 'sshKeys'

// Top-to-bottom order of the sections below — shared by `sectionRef` and the
// scroll-spy so adding/reordering a section only needs a change here. Account
// leads — it's the most prominent (koma.run sign-in is otherwise buried in the
// Connector panel's OAuth list).
const SECTION_ORDER: SectionId[] = [
  'account',
  'appearance',
  'session',
  'activityBar',
  'lsp',
  'sshKeys',
]

export default function SettingsTab() {
  const req = useKoma((s) => s.req)
  const activeTheme = useKoma((s) => s.config.theme)
  const palettes = useKoma((s) => s.config.palettes)
  const themes = useKoma((s) => s.config.themes)

  const scrollRef = useRef<HTMLDivElement>(null)
  const accountRef = useRef<HTMLDivElement>(null)
  const appearanceRef = useRef<HTMLDivElement>(null)
  const sessionRef = useRef<HTMLDivElement>(null)
  const activityBarRef = useRef<HTMLDivElement>(null)
  const lspRef = useRef<HTMLDivElement>(null)
  const sshKeysRef = useRef<HTMLDivElement>(null)
  const [active, setActive] = useState<SectionId>('account')

  const sectionRef = (id: SectionId) =>
    id === 'account'
      ? accountRef
      : id === 'appearance'
        ? appearanceRef
        : id === 'session'
          ? sessionRef
          : id === 'activityBar'
            ? activityBarRef
            : id === 'lsp'
              ? lspRef
              : sshKeysRef

  // Nav click → smooth-scroll the pane to the section header.
  const goto = (id: SectionId) => {
    setActive(id)
    sectionRef(id).current?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  }

  // Scroll spy: highlight the nav item whose section is at the top of the pane.
  // Uses viewport rects (robust against layout offsets) — walking sections
  // top-to-bottom and keeping the LAST one whose header has crossed ~80px below
  // the pane's top edge means a later section always beats an earlier one.
  const onScroll = () => {
    const pane = scrollRef.current
    if (!pane) return
    const paneTop = pane.getBoundingClientRect().top
    let current: SectionId = 'account'
    for (const id of SECTION_ORDER) {
      const el = sectionRef(id).current
      if (!el) continue
      const delta = el.getBoundingClientRect().top - paneTop
      if (delta < 80) current = id
    }
    setActive(current)
  }

  return (
    <div className="flex h-full w-full min-w-0 bg-koma-bg text-koma-fg">
      <nav className="flex w-40 flex-none flex-col gap-0.5 border-r border-koma-border bg-koma-panel2 p-2">
        <div className="px-2 pb-1.5 pt-1 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-40">
          Settings
        </div>
        <NavItem
          icon={<UserCircle size={15} />}
          label="Account"
          active={active === 'account'}
          onClick={() => goto('account')}
        />
        <NavItem
          icon={<PaletteIcon size={15} />}
          label="Appearance"
          active={active === 'appearance'}
          onClick={() => goto('appearance')}
        />
        <NavItem
          icon={<SlidersHorizontal size={15} />}
          label="Session"
          active={active === 'session'}
          onClick={() => goto('session')}
        />
        <NavItem
          icon={<PanelLeft size={15} />}
          label="Sidebar"
          active={active === 'activityBar'}
          onClick={() => goto('activityBar')}
        />
        <NavItem
          icon={<Code2 size={15} />}
          label="Language servers"
          active={active === 'lsp'}
          onClick={() => goto('lsp')}
        />
        <NavItem
          icon={<KeyRound size={15} />}
          label="SSH Keys"
          active={active === 'sshKeys'}
          onClick={() => goto('sshKeys')}
        />
      </nav>

      <div ref={scrollRef} onScroll={onScroll} className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl px-8 py-6">
          <section ref={accountRef}>
            <SectionHeader title="Account" desc="Sign in with koma.run — unlocks the extension store." />
            <AccountSettings />
          </section>

          <section ref={appearanceRef} className="mt-12">
            <SectionHeader title="Appearance" desc="Pick a colour theme — it applies instantly across the whole app." />
            <PaletteGrid
              palettes={palettes}
              themes={themes}
              active={activeTheme}
              onPick={(name) => req({ r: 'SetTheme', name })}
            />
          </section>

          <section ref={sessionRef} className="mt-12">
            <SectionHeader title="Session" desc="Preferences for the current session." />
            <SessionSettings />
          </section>

          <section ref={activityBarRef} className="mt-12">
            <SectionHeader
              title="Sidebar"
              desc="Show or hide activity-bar icons. Hidden icons move into the “…” overflow menu instead of disappearing — drag an icon on the activity bar itself to reorder it."
            />
            <ActivityBarSettings />
          </section>

          <section ref={lspRef} className="mt-12">
            <SectionHeader
              title="Language servers"
              desc="Optional language servers for the coding panel. koma-managed installs land under ~/.koma/lsp/ and never touch system packages. PATH copies are detected automatically."
            />
            <LspSettings />
          </section>

          <section ref={sshKeysRef} className="mt-12">
            <SectionHeader
              title="SSH Keys"
              desc="A local key vault for git remotes — generate or import keys and manage them here. Separate from the agent's own credentials."
            />
            <SshKeysSettings />
          </section>
        </div>
      </div>
    </div>
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

// ── Account (koma.run) ───────────────────────────────────────────────────────
// Surfaces the koma.run account sign-in that otherwise only lives buried in
// the Connector panel's OAuth provider list — same OAuth wire (`oauth` slice,
// StartOAuth/CancelOAuth/DeleteOAuthConn/GetOAuthState), scoped to just the
// "komarun" provider (wire id for `OAuthProvider::KomaRun`, label "Koma"). No
// in-app profile/dashboard — sign-in state + a hyperlink out to koma.run.

const KOMARUN_PROVIDER = 'komarun'
const KOMARUN_DASHBOARD_URL = 'https://koma.run/dashboard'

function AccountSettings() {
  const req = useKoma((s) => s.req)
  const openExternal = useKoma((s) => s.openExternal)
  const conns = useKoma((s) => s.oauth.conns)
  const phase = useKoma((s) => s.oauth.phase)
  const oauthUrl = useKoma((s) => s.oauth.url)

  // Refresh on mount so the section reflects current state whether it was
  // populated already (e.g. Connector panel was opened earlier this session)
  // or not.
  useEffect(() => {
    req({ r: 'GetOAuthState' })
  }, [req])

  const conn = conns.find((c) => c.provider === KOMARUN_PROVIDER) ?? null

  // The `oauth` slice is a single global flow (one at a time) shared with the
  // Connector panel's OAuthConnect — track locally whether THIS section is
  // the one that started it, so a flow started elsewhere (a different
  // provider) never makes this section render an in-flight status that isn't
  // actually its own.
  const [started, setStarted] = useState(false)
  useEffect(() => {
    if (phase === 'success' || phase === 'idle' || phase === 'failed') setStarted(false)
  }, [phase])

  const inFlight = started && (phase === 'starting' || phase === 'waiting_url')

  const signIn = () => {
    setStarted(true)
    req({ r: 'StartOAuth', provider: KOMARUN_PROVIDER })
  }
  const cancel = () => {
    req({ r: 'CancelOAuth' })
    setStarted(false)
  }
  const signOut = () => {
    if (conn) req({ r: 'DeleteOAuthConn', uuid: conn.uuid })
  }

  return (
    <div className="flex flex-col gap-3">
      {inFlight ? (
        <div className="flex flex-col gap-2 rounded border border-koma-border bg-koma-panel2 px-3 py-2.5">
          <div className="flex items-center gap-2 text-[12.5px] text-koma-fg">
            <BrailleSpinner size={13} className="opacity-70" />
            Opening your browser… complete sign-in there.
          </div>
          {oauthUrl && (
            <div className="truncate font-mono text-[11px] text-koma-fg opacity-50">{oauthUrl}</div>
          )}
          <div>
            <button
              type="button"
              onClick={cancel}
              className="rounded px-2 py-1 text-[12px] text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
            >
              Cancel
            </button>
          </div>
        </div>
      ) : conn ? (
        <div className="flex items-center justify-between gap-3 rounded border border-koma-border bg-koma-panel2 px-3 py-2.5">
          <div className="flex min-w-0 items-center gap-2.5">
            <span className="h-2 w-2 flex-none rounded-full bg-koma-success" title="Connected" />
            <div className="min-w-0">
              <div className="truncate text-[13px] text-koma-fg">
                Signed in as {conn.email || conn.name || 'koma.run account'}
              </div>
              <div className="text-[11px] text-koma-fg opacity-45">Connected</div>
            </div>
          </div>
          <button
            type="button"
            onClick={signOut}
            className="flex-none rounded border border-koma-border px-2.5 py-1 text-[12px] text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100"
          >
            Sign out
          </button>
        </div>
      ) : (
        <div className="flex items-center justify-between gap-3 rounded border border-koma-border bg-koma-panel2 px-3 py-2.5">
          <div className="min-w-0 flex-1">
            <div className="text-[13px] text-koma-fg">Not signed in to koma.run</div>
            <div className="mt-0.5 text-[11.5px] leading-snug text-koma-fg opacity-45">
              A koma.run account unlocks the extension store.
            </div>
          </div>
          <button
            type="button"
            onClick={signIn}
            className="flex-none rounded bg-koma-accent px-3 py-1.5 text-[12px] font-semibold text-koma-bg"
          >
            Sign in to koma.run
          </button>
        </div>
      )}

      <a
        href={KOMARUN_DASHBOARD_URL}
        onClick={(e) => {
          e.preventDefault()
          openExternal(KOMARUN_DASHBOARD_URL)
        }}
        className="flex w-fit items-center gap-1 text-[12px] text-koma-accent opacity-80 hover:opacity-100 hover:underline"
      >
        Manage account on koma.run
        <ExternalLink size={12} />
      </a>
    </div>
  )
}

// ── Appearance ──────────────────────────────────────────────────────────────

function PaletteGrid({
  palettes,
  themes,
  active,
  onPick,
}: {
  palettes: PaletteInfo[]
  themes: string[]
  active: string
  onPick: (name: string) => void
}) {
  // Prefer the rich colour catalogue; degrade to names-only cards (no strip) for
  // a host build that doesn't project `palettes` yet.
  const cards: PaletteInfo[] =
    palettes.length > 0 ? palettes : themes.map((name) => ({ name, colors: [] }))

  return (
    <div className="grid gap-3 [grid-template-columns:repeat(auto-fill,minmax(190px,1fr))]">
      {cards.map((p) => (
        <PaletteCard
          key={p.name}
          name={p.name}
          colors={p.colors}
          active={p.name === active}
          onPick={() => onPick(p.name)}
        />
      ))}
    </div>
  )
}

function PaletteCard({
  name,
  colors,
  active,
  onPick,
}: {
  name: string
  colors: string[]
  active: boolean
  onPick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onPick}
      title={name}
      className={`group relative flex flex-col gap-2 rounded-lg border p-2 text-left transition-all duration-150 hover:-translate-y-0.5 ${
        active
          ? 'border-koma-accent bg-koma-hover ring-2 ring-koma-accent/60'
          : 'border-koma-border bg-koma-panel2 hover:border-koma-grip'
      }`}
    >
      {active && (
        <span className="absolute right-2 top-2 z-10 flex h-4 w-4 items-center justify-center rounded-full bg-koma-accent text-koma-bg">
          <Check size={11} strokeWidth={3} />
        </span>
      )}
      {/* Movie-palette strip: the 11 role colours as contiguous equal blocks. */}
      <div className="flex h-9 overflow-hidden rounded-md border border-koma-border">
        {colors.length > 0 ? (
          colors.map((c, i) => (
            <span
              key={i}
              className="flex-1 transition-[filter] group-hover:brightness-110"
              style={{ backgroundColor: c }}
            />
          ))
        ) : (
          <span className="flex-1 bg-koma-panel" />
        )}
      </div>
      <span className="truncate font-mono text-[11.5px] text-koma-fg opacity-80">{name}</span>
    </button>
  )
}

// ── Session ─────────────────────────────────────────────────────────────────

function SessionSettings() {
  const req = useKoma((s) => s.req)
  const values = useKoma((s) => s.settingsValues)

  // Local editable mirrors of the authoritative values. Re-synced whenever a
  // fresh SettingsValues reply lands (GetSettings on open/activate, or the
  // re-push after a SetPrefs). Clicking any control blurs a focused input first,
  // committing its edit before the toggle's re-push arrives — so the sync never
  // clobbers an in-progress name/workdir edit.
  const [name, setName] = useState('')
  const [workdir, setWorkdir] = useState('')
  const [shortSend, setShortSend] = useState(true)
  const [slidingCache, setSlidingCache] = useState(false)
  const [bashSaving, setBashSaving] = useState(true)
  const [codingAutosave, setCodingAutosave] = useState(false)
  const [internet, setInternet] = useState<'simple' | 'full'>('simple')
  const [maxTurns, setMaxTurns] = useState('500')

  useEffect(() => {
    if (!values) return
    setName(values.name)
    setWorkdir(values.workdir.join('\n'))
    setShortSend(values.shortSend)
    setSlidingCache(values.slidingCache)
    setBashSaving(values.bashSaving)
    setCodingAutosave(values.codingAutosave)
    setInternet(values.internetMode === 'full' ? 'full' : 'simple')
    setMaxTurns(String(values.subagentMaxTurns ?? 500))
  }, [values])

  if (!values) {
    return (
      <div className="flex items-center gap-2 py-6 text-[12px] text-koma-fg opacity-45">
        <BrailleSpinner size={14} className="opacity-70" />
        Loading session settings…
      </div>
    )
  }

  // Name: commit on blur / Enter, skipping an empty or unchanged value.
  const commitName = () => {
    const n = name.trim()
    if (!n || n === values.name) return
    req({ r: 'Rename', name: n })
  }
  // Workdir: commit on blur, one dir per line; skip when unchanged vs the
  // authoritative list (the daemon re-normalises + re-pushes anyway).
  const commitWorkdir = () => {
    const lines = workdir
      .split('\n')
      .map((l) => l.trim())
      .filter(Boolean)
    if (JSON.stringify(lines) === JSON.stringify(values.workdir)) return
    req({ r: 'SetPrefs', workdir: lines })
  }
  // Toggles / internet: optimistic flip + fire — the re-push corrects if needed.
  const setShort = (v: boolean) => {
    setShortSend(v)
    req({ r: 'SetPrefs', shortSend: v })
  }
  const setSliding = (v: boolean) => {
    setSlidingCache(v)
    req({ r: 'SetPrefs', slidingCache: v })
  }
  const setBash = (v: boolean) => {
    setBashSaving(v)
    req({ r: 'SetPrefs', bashSaving: v })
  }
  const setCodingAuto = (v: boolean) => {
    setCodingAutosave(v)
    req({ r: 'SetPrefs', codingAutosave: v })
  }
  const setNet = (v: 'simple' | 'full') => {
    setInternet(v)
    req({ r: 'SetPrefs', internetMode: v })
  }
  // Max turns: commit on blur; clamp ≥ 1, fall back to 500 on invalid input.
  const commitMaxTurns = () => {
    const n = parseInt(maxTurns, 10)
    const safe = isNaN(n) || n < 1 ? 500 : n
    setMaxTurns(String(safe))
    if (safe !== (values.subagentMaxTurns ?? 500)) {
      req({ r: 'SetPrefs', subagentMaxTurns: safe })
    }
  }

  return (
    <div className="flex flex-col">
      <SettingRow label="Name" desc="The session's display name, shown in the switcher.">
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          onBlur={commitName}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault()
              ;(e.target as HTMLInputElement).blur()
            }
          }}
          placeholder="untitled session"
          className="h-7 w-56 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-35 focus:border-koma-grip"
        />
      </SettingRow>

      <SettingRow
        label="Working directories"
        desc="One directory per line. The first is the primary workspace root; the rest widen the harness allow-set."
        align="start"
      >
        <textarea
          value={workdir}
          onChange={(e) => setWorkdir(e.target.value)}
          onBlur={commitWorkdir}
          rows={3}
          spellCheck={false}
          placeholder="/path/to/project"
          className="h-20 w-72 resize-y rounded border border-koma-border bg-koma-bg px-2 py-1.5 font-mono text-[11.5px] leading-relaxed text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-35 focus:border-koma-grip"
        />
      </SettingRow>

      <SettingRow label="Short-send" desc="Compress older turns into a rolling summary before each send to cut token cost.">
        <Toggle on={shortSend} onChange={setShort} />
      </SettingRow>

      <SettingRow label="Sliding cache" desc="Adapt summarisation when the provider's prompt cache goes cold (e.g. Anthropic).">
        <Toggle on={slidingCache} onChange={setSliding} />
      </SettingRow>

      <SettingRow label="Bash shorts" desc="Filter and tee bash / git output to disk to preserve command logs.">
        <Toggle on={bashSaving} onChange={setBash} />
      </SettingRow>

      <SettingRow label="Coding autosave" desc="Debounced auto-save for Coding panel editor tabs (750ms after edits stop).">
        <Toggle on={codingAutosave} onChange={setCodingAuto} />
      </SettingRow>

      <SettingRow label="Internet mode" desc="Full upgrades web_fetch to the browser backend (renders JS, higher token use).">
        <div className="w-40">
          <Segmented
            value={internet}
            options={[
              { value: 'simple', label: 'Simple' },
              { value: 'full', label: 'Full' },
            ]}
            onChange={setNet}
          />
        </div>
      </SettingRow>

      <SettingRow label="Max turns" desc="Max agentic turns per sub-agent (when agent has no step cap). Default: 500.">
        <input
          type="number"
          min={1}
          value={maxTurns}
          onChange={(e) => setMaxTurns(e.target.value.replace(/[^0-9]/g, ''))}
          onBlur={commitMaxTurns}
          onKeyDown={(e) => {
            if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
          }}
          className="w-24 rounded border border-koma-border bg-koma-bg px-2 py-1.5 font-mono text-[12px] text-koma-fg outline-none focus:border-koma-grip"
        />
      </SettingRow>
    </div>
  )
}

function SettingRow({
  label,
  desc,
  children,
  align = 'center',
}: {
  label: string
  desc?: string
  children: ReactNode
  align?: 'start' | 'center'
}) {
  return (
    <div
      className={`flex justify-between gap-6 border-b border-koma-border py-3.5 ${
        align === 'start' ? 'items-start' : 'items-center'
      }`}
    >
      <div className="min-w-0 flex-1">
        <div className="text-[13px] text-koma-fg">{label}</div>
        {desc && <div className="mt-0.5 text-[11.5px] leading-snug text-koma-fg opacity-45">{desc}</div>}
      </div>
      <div className="flex-none">{children}</div>
    </div>
  )
}

// ── Sidebar (ActivityBar layout) ─────────────────────────────────────────────
// Per-icon visibility toggles for the built-in activity-bar items
// (ACTIVITY_BAR_ITEMS, shared with ActivityBar.tsx). Reordering itself only
// happens by dragging on the bar; this list just mirrors the effective order
// so the toggle a user reaches for is where they'd expect it after a drag.
// Toggling here writes through the same `activityBar` store slice the bar
// reads (setActivityBarHidden), persisted to localStorage — no host round trip.

function ActivityBarSettings() {
  const order = useKoma((s) => s.activityBar.order)
  const hidden = useKoma((s) => s.activityBar.hidden)
  const setActivityBarHidden = useKoma((s) => s.setActivityBarHidden)

  const allIds = ACTIVITY_BAR_ITEMS.map((i) => i.view)
  const effectiveOrder = resolveActivityBarOrder(order, allIds)
  const itemByView = new Map<string, (typeof ACTIVITY_BAR_ITEMS)[number]>(
    ACTIVITY_BAR_ITEMS.map((i) => [i.view, i]),
  )
  const hiddenSet = new Set(hidden)

  return (
    <div className="flex flex-col">
      {effectiveOrder.map((view) => {
        const item = itemByView.get(view)
        if (!item) return null
        return (
          <ActivityBarRow
            key={view}
            icon={item.icon}
            label={item.label}
            visible={!hiddenSet.has(view)}
            onToggle={(v) => setActivityBarHidden(view, !v)}
          />
        )
      })}
    </div>
  )
}

function ActivityBarRow({
  icon: Icon,
  label,
  visible,
  onToggle,
}: {
  icon: LucideIcon
  label: string
  visible: boolean
  onToggle: (v: boolean) => void
}) {
  return (
    <div className="flex items-center justify-between gap-6 border-b border-koma-border py-3">
      <div className="flex min-w-0 flex-1 items-center gap-2.5">
        <Icon size={15} className="flex-none text-koma-fg opacity-60" />
        <span className="truncate text-[13px] text-koma-fg">{label}</span>
      </div>
      <Toggle on={visible} onChange={onToggle} />
    </div>
  )
}

// ── SSH Keys ────────────────────────────────────────────────────────────────
// A GUI-only, MANUAL key vault (`<~/.koma>/keys/`) the user owns directly —
// entirely separate from the model's own git credential machinery. Wave 4a:
// list / generate / import / reveal (copy-public, reveal-private) / delete.
// Remote push/pull (wave 4b) is not wired here. Host-req-driven, like the GIT
// panel — NOT the daemon-backed SettingsValues flow the Session section uses.

function SshKeysSettings() {
  const keys = useKoma((s) => s.keys)
  const revealResult = useKoma((s) => s.keyRevealResult)
  const refreshKeys = useKoma((s) => s.refreshKeys)
  const keyGenerate = useKoma((s) => s.keyGenerate)
  const keyImport = useKoma((s) => s.keyImport)
  const keyReveal = useKoma((s) => s.keyReveal)
  const clearKeyReveal = useKoma((s) => s.clearKeyReveal)
  const keyDelete = useKoma((s) => s.keyDelete)

  // Fetch fresh list on mount (section scrolled into view for the first time —
  // it's always mounted alongside Appearance/Session, so this fires once per
  // tab-open, mirroring GitPanel's refresh-on-mount).
  useEffect(() => {
    refreshKeys()
  }, [refreshKeys])

  const [showGenerate, setShowGenerate] = useState(false)
  const [genName, setGenName] = useState('')
  const [genComment, setGenComment] = useState('')
  const [showImport, setShowImport] = useState(false)
  const [impName, setImpName] = useState('')
  const [impKey, setImpKey] = useState('')
  const [armedDelete, setArmedDelete] = useState<string | null>(null)
  const [armedReveal, setArmedReveal] = useState<string | null>(null)
  // "Copy public key" needs no click-gate (it's non-sensitive), but IS async —
  // the reveal round-trips through the host. `copyPending` names whose reply
  // to auto-copy the instant it lands; `copiedName` drives a transient
  // check-mark swap on that row's button.
  const [copyPending, setCopyPending] = useState<string | null>(null)
  const [copiedName, setCopiedName] = useState<string | null>(null)

  useEffect(() => {
    if (!revealResult || revealResult.private || revealResult.name !== copyPending) return
    if (!revealResult.error) {
      void navigator.clipboard?.writeText(revealResult.content).catch(() => {})
      setCopiedName(revealResult.name)
      window.setTimeout(() => setCopiedName((n) => (n === revealResult.name ? null : n)), 1500)
    }
    setCopyPending(null)
    clearKeyReveal()
  }, [revealResult, copyPending, clearKeyReveal])

  const submitGenerate = () => {
    if (!genName.trim()) return
    keyGenerate(genName.trim(), genComment)
    setGenName('')
    setGenComment('')
    setShowGenerate(false)
  }

  const submitImport = () => {
    if (!impName.trim() || !impKey.trim()) return
    keyImport(impName.trim(), impKey)
    setImpName('')
    setImpKey('')
    setShowImport(false)
  }

  // There is only ONE transient reveal slot store-wide (see `keyRevealResult`),
  // so only one key's PRIVATE content is ever shown at a time — revealing a
  // different key's replaces it. The public-copy path above never lands here
  // (it clears the slot the instant it copies).
  const revealedPrivate = revealResult && revealResult.private ? revealResult : null

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={() => {
            setShowImport(false)
            setShowGenerate((v) => !v)
          }}
          className="flex items-center gap-1.5 rounded border border-koma-border px-2.5 py-1 text-[12px] text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100"
        >
          <Plus size={13} />
          Generate
        </button>
        <button
          type="button"
          onClick={() => {
            setShowGenerate(false)
            setShowImport((v) => !v)
          }}
          className="flex items-center gap-1.5 rounded border border-koma-border px-2.5 py-1 text-[12px] text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100"
        >
          <Plus size={13} />
          Import
        </button>
      </div>

      {showGenerate && (
        <div className="flex flex-col gap-2 rounded border border-koma-border bg-koma-panel2 py-2">
          <Field label="Name">
            <TextInput
              value={genName}
              onChange={(e) => setGenName(e.target.value)}
              placeholder="e.g. github-deploy"
              autoFocus
            />
          </Field>
          <Field label="Comment">
            <TextInput
              value={genComment}
              onChange={(e) => setGenComment(e.target.value)}
              placeholder="koma"
            />
          </Field>
          <div className="flex justify-end gap-2 px-3 pb-1">
            <button
              type="button"
              onClick={() => setShowGenerate(false)}
              className="rounded px-2 py-1 text-[12px] text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
            >
              Cancel
            </button>
            <button
              type="button"
              disabled={!genName.trim()}
              onClick={submitGenerate}
              className="rounded bg-koma-accent px-3 py-1 text-[12px] font-semibold text-koma-bg disabled:cursor-not-allowed disabled:opacity-35"
            >
              Generate
            </button>
          </div>
        </div>
      )}

      {showImport && (
        <div className="flex flex-col gap-2 rounded border border-koma-border bg-koma-panel2 py-2">
          <Field label="Name">
            <TextInput
              value={impName}
              onChange={(e) => setImpName(e.target.value)}
              placeholder="e.g. github-deploy"
              autoFocus
            />
          </Field>
          <Field label="Private key">
            <textarea
              value={impKey}
              onChange={(e) => setImpKey(e.target.value)}
              rows={6}
              spellCheck={false}
              placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
              className="w-full resize-y rounded border border-koma-border bg-koma-bg px-2 py-1.5 font-mono text-[11px] leading-relaxed text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-35 focus:border-koma-grip"
            />
          </Field>
          <div className="flex justify-end gap-2 px-3 pb-1">
            <button
              type="button"
              onClick={() => setShowImport(false)}
              className="rounded px-2 py-1 text-[12px] text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
            >
              Cancel
            </button>
            <button
              type="button"
              disabled={!impName.trim() || !impKey.trim()}
              onClick={submitImport}
              className="rounded bg-koma-accent px-3 py-1 text-[12px] font-semibold text-koma-bg disabled:cursor-not-allowed disabled:opacity-35"
            >
              Import
            </button>
          </div>
        </div>
      )}

      {keys.length === 0 ? (
        <div className="py-6 text-center text-[12px] text-koma-fg opacity-45">
          No keys yet — generate or import one above.
        </div>
      ) : (
        <div className="flex flex-col divide-y divide-koma-border rounded border border-koma-border">
          {keys.map((k) => (
            <div key={k.name}>
              {armedDelete === k.name ? (
                <KeyConfirmRow
                  label={`Delete key "${k.name}"?`}
                  confirmLabel="delete"
                  onConfirm={() => {
                    keyDelete(k.name)
                    setArmedDelete(null)
                  }}
                  onCancel={() => setArmedDelete(null)}
                />
              ) : armedReveal === k.name ? (
                <KeyConfirmRow
                  label={`Reveal "${k.name}"'s PRIVATE key? Anyone with it can act as you.`}
                  confirmLabel="reveal"
                  onConfirm={() => {
                    keyReveal(k.name, true)
                    setArmedReveal(null)
                  }}
                  onCancel={() => setArmedReveal(null)}
                />
              ) : (
                <div className="flex items-center gap-2 px-3 py-2">
                  <KeyRound size={14} className="flex-none text-koma-fg opacity-45" />
                  <div className="min-w-0 flex-1">
                    <div className="truncate font-mono text-[12px] text-koma-fg">{k.name}</div>
                    <div className="truncate text-[11px] text-koma-fg opacity-45">
                      {k.keyType && <span>{k.keyType} · </span>}
                      {k.fingerprint}
                      {k.comment && <span> · {k.comment}</span>}
                    </div>
                  </div>
                  <button
                    type="button"
                    title="Copy public key"
                    aria-label="Copy public key"
                    onClick={() => {
                      setCopyPending(k.name)
                      keyReveal(k.name, false)
                    }}
                    className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-fg opacity-60 hover:bg-koma-hover hover:opacity-100"
                  >
                    {copiedName === k.name ? <Check size={13} /> : <Copy size={13} />}
                  </button>
                  <button
                    type="button"
                    title="Reveal private key"
                    aria-label="Reveal private key"
                    onClick={() => setArmedReveal(k.name)}
                    className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-fg opacity-60 hover:bg-koma-hover hover:opacity-100"
                  >
                    <Eye size={13} />
                  </button>
                  <button
                    type="button"
                    title="Delete key"
                    aria-label="Delete key"
                    onClick={() => setArmedDelete(k.name)}
                    className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-fg opacity-60 hover:bg-koma-hover hover:text-koma-error hover:opacity-100"
                  >
                    <Trash2 size={13} />
                  </button>
                </div>
              )}
              {revealedPrivate && revealedPrivate.name === k.name && (
                <div className="border-t border-koma-error/30 bg-koma-error/5 px-3 py-2">
                  <div className="mb-1 flex items-center justify-between">
                    <span className="text-[11px] font-semibold text-koma-error">
                      Private key — keep this secret
                    </span>
                    <button
                      type="button"
                      onClick={() => clearKeyReveal()}
                      className="rounded px-1.5 py-0.5 text-[11px] text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
                    >
                      Hide
                    </button>
                  </div>
                  {revealedPrivate.error ? (
                    <div className="text-[11px] text-koma-error">{revealedPrivate.error}</div>
                  ) : (
                    <textarea
                      readOnly
                      value={revealedPrivate.content}
                      rows={6}
                      spellCheck={false}
                      onFocus={(e) => e.target.select()}
                      className="w-full resize-y rounded border border-koma-border bg-koma-bg px-2 py-1.5 font-mono text-[11px] leading-relaxed text-koma-fg outline-none"
                    />
                  )}
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// Small inline confirm row, mirroring GitPanel's DiscardConfirmRow idiom (a
// click-to-confirm — NEVER window.confirm) for the SSH Keys section's
// destructive/sensitive actions (delete, private-key reveal).
function KeyConfirmRow({
  label,
  confirmLabel,
  onConfirm,
  onCancel,
}: {
  label: string
  confirmLabel: string
  onConfirm: () => void
  onCancel: () => void
}) {
  return (
    <div className="flex min-h-[30px] items-center justify-between gap-2 bg-koma-error/10 px-3 py-1.5 text-[12px] font-medium text-koma-error">
      <span className="min-w-0 flex-1 truncate">{label}</span>
      <span className="flex flex-none items-center gap-1">
        <button
          type="button"
          autoFocus
          onClick={onConfirm}
          aria-label={`Confirm ${confirmLabel}`}
          className="flex flex-none items-center gap-1 rounded px-2 py-0.5 font-semibold opacity-90 hover:bg-koma-hover hover:opacity-100"
        >
          <Check size={12} className="flex-none" />
          {confirmLabel}
        </button>
        <button
          type="button"
          onClick={onCancel}
          aria-label="Cancel"
          className="flex flex-none items-center gap-1 rounded px-2 py-0.5 text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
        >
          <X size={12} className="flex-none" />
          cancel
        </button>
      </span>
    </div>
  )
}

// ── Language servers ─────────────────────────────────────────────────────────
// Catalogue of language servers for the coding panel. Host-local under
// ~/.koma/lsp/; PATH copies are detected. Install never mutates system
// packages. Matches the SSH Keys host-req pattern (refresh on mount).

function LspSettings() {
  const servers = useKoma((s) => s.lspServers)
  const progress = useKoma((s) => s.lspProgress)
  const refreshLsp = useKoma((s) => s.refreshLsp)
  const lspInstall = useKoma((s) => s.lspInstall)
  const lspUninstall = useKoma((s) => s.lspUninstall)
  const [armedUninstall, setArmedUninstall] = useState<string | null>(null)

  useEffect(() => {
    refreshLsp()
  }, [refreshLsp])

  const anyInstalling = Object.values(progress).some((p) => p.pct < 100 && !p.error)

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          disabled={anyInstalling}
          onClick={() => lspInstall(null, true, false)}
          className="flex items-center gap-1.5 rounded border border-koma-border px-2.5 py-1 text-[12px] text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100 disabled:opacity-40"
        >
          {anyInstalling ? <BrailleSpinner size={13} /> : <Download size={13} />}
          Install all
        </button>
        <button
          type="button"
          onClick={() => refreshLsp()}
          className="flex items-center gap-1.5 rounded border border-koma-border px-2.5 py-1 text-[12px] text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100"
        >
          Refresh
        </button>
      </div>

      {servers.length === 0 ? (
        <div className="py-6 text-center text-[12px] text-koma-fg opacity-45">
          Loading language-server catalogue…
        </div>
      ) : (
        <div className="flex flex-col">
          {servers.map((s) => {
            const prog = progress[s.id]
            const installing = !!prog && prog.pct < 100 && !prog.error
            const sourceLabel =
              s.source === 'managed' ? 'koma-managed' : s.source === 'path' ? 'on PATH' : 'missing'
            const sourceTone =
              s.source === 'managed'
                ? 'text-koma-success'
                : s.source === 'path'
                  ? 'text-koma-accent'
                  : 'text-koma-dim opacity-70'
            return (
              <div
                key={s.id}
                className="flex flex-col gap-1.5 border-b border-koma-border py-3"
              >
                <div className="flex items-start justify-between gap-4">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-[13px] font-medium text-koma-fg">{s.name}</span>
                      <span className={`text-[11px] ${sourceTone}`}>{sourceLabel}</span>
                      {s.version && (
                        <span className="text-[11px] text-koma-fg opacity-40">{s.version}</span>
                      )}
                    </div>
                    <div className="mt-0.5 text-[11.5px] text-koma-fg opacity-45">
                      <code className="opacity-80">{s.binary}</code>
                      {' · '}
                      {s.installKind}
                      {s.extensions.length > 0 && (
                        <> · {s.extensions.map((e) => `.${e}`).join(' ')}</>
                      )}
                    </div>
                    {prog && !prog.error && prog.pct < 100 && (
                      <div className="mt-1 text-[11px] text-koma-accent">
                        Installing… {prog.pct}%
                      </div>
                    )}
                    {prog?.error && (
                      <div className="mt-1 text-[11px] text-koma-error">{prog.error}</div>
                    )}
                  </div>
                  <div className="flex flex-none items-center gap-1.5">
                    {s.source === 'managed' ? (
                      armedUninstall === s.id ? (
                        <KeyConfirmRow
                          label={`Uninstall ${s.id}?`}
                          confirmLabel="Uninstall"
                          onConfirm={() => {
                            lspUninstall(s.id)
                            setArmedUninstall(null)
                          }}
                          onCancel={() => setArmedUninstall(null)}
                        />
                      ) : (
                        <>
                          <button
                            type="button"
                            disabled={installing}
                            onClick={() => lspInstall(s.id, false, true)}
                            className="rounded border border-koma-border px-2 py-0.5 text-[11.5px] text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100 disabled:opacity-40"
                          >
                            Update
                          </button>
                          <button
                            type="button"
                            disabled={installing}
                            onClick={() => setArmedUninstall(s.id)}
                            className="rounded border border-koma-border px-2 py-0.5 text-[11.5px] text-koma-error opacity-80 hover:bg-koma-hover hover:opacity-100 disabled:opacity-40"
                          >
                            Uninstall
                          </button>
                        </>
                      )
                    ) : s.source === 'path' ? (
                      <button
                        type="button"
                        disabled={installing}
                        onClick={() => lspInstall(s.id, false, false)}
                        className="rounded border border-koma-border px-2 py-0.5 text-[11.5px] text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100 disabled:opacity-40"
                        title="Also install a koma-managed copy under ~/.koma/lsp/"
                      >
                        Install managed
                      </button>
                    ) : (
                      <button
                        type="button"
                        disabled={installing}
                        onClick={() => lspInstall(s.id, false, false)}
                        className="flex items-center gap-1 rounded border border-koma-accent/40 bg-koma-accent/10 px-2 py-0.5 text-[11.5px] text-koma-accent hover:bg-koma-accent/20 disabled:opacity-40"
                      >
                        {installing ? (
                          <BrailleSpinner size={12} />
                        ) : (
                          <Download size={12} />
                        )}
                        Install
                      </button>
                    )}
                  </div>
                </div>
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}
