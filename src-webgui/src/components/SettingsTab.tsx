import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Check, Loader2, Palette as PaletteIcon, SlidersHorizontal } from 'lucide-react'
import { useKoma, type PaletteInfo } from '../store/koma'
import { Segmented, Toggle } from './panels/form'

// VSCode-style Settings page, rendered as a tab over the main content column
// (see routes/index.tsx TabbedMain). A left nav rail scrolls the content pane to
// its two sections; Appearance is a movie-strip palette grid, Session is a
// labelled list of the session prefs. Every colour is a theme token
// (var(--koma-*) via the koma-* Tailwind classes) so it tracks the live palette.

type SectionId = 'appearance' | 'session'

export default function SettingsTab() {
  const req = useKoma((s) => s.req)
  const activeTheme = useKoma((s) => s.config.theme)
  const palettes = useKoma((s) => s.config.palettes)
  const themes = useKoma((s) => s.config.themes)

  const scrollRef = useRef<HTMLDivElement>(null)
  const appearanceRef = useRef<HTMLDivElement>(null)
  const sessionRef = useRef<HTMLDivElement>(null)
  const [active, setActive] = useState<SectionId>('appearance')

  // Nav click → smooth-scroll the pane to the section header.
  const goto = (id: SectionId) => {
    setActive(id)
    const el = id === 'appearance' ? appearanceRef.current : sessionRef.current
    el?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  }

  // Scroll spy: highlight the nav item whose section is at the top of the pane.
  // Uses viewport rects (robust against layout offsets) — Session wins once its
  // header crosses ~80px below the pane's top edge.
  const onScroll = () => {
    const pane = scrollRef.current
    const sess = sessionRef.current
    if (!pane || !sess) return
    const delta = sess.getBoundingClientRect().top - pane.getBoundingClientRect().top
    setActive(delta < 80 ? 'session' : 'appearance')
  }

  return (
    <div className="flex h-full w-full min-w-0 bg-koma-bg text-koma-fg">
      <nav className="flex w-40 flex-none flex-col gap-0.5 border-r border-koma-border bg-koma-panel2 p-2">
        <div className="px-2 pb-1.5 pt-1 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-40">
          Settings
        </div>
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
      </nav>

      <div ref={scrollRef} onScroll={onScroll} className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl px-8 py-6">
          <section ref={appearanceRef}>
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
  const [internet, setInternet] = useState<'simple' | 'full'>('simple')

  useEffect(() => {
    if (!values) return
    setName(values.name)
    setWorkdir(values.workdir.join('\n'))
    setShortSend(values.shortSend)
    setSlidingCache(values.slidingCache)
    setBashSaving(values.bashSaving)
    setInternet(values.internetMode === 'full' ? 'full' : 'simple')
  }, [values])

  if (!values) {
    return (
      <div className="flex items-center gap-2 py-6 text-[12px] text-koma-fg opacity-45">
        <Loader2 size={14} className="animate-spin opacity-70" />
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
  const setNet = (v: 'simple' | 'full') => {
    setInternet(v)
    req({ r: 'SetPrefs', internetMode: v })
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
