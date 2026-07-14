import { useEffect } from 'react'
import { Package, Blocks } from 'lucide-react'
import { BrailleSpinner } from './BrailleSpinner'
import { useKoma } from '../store/koma'

// Shared helpers — same shapes as StoreTab for visual parity.
function TierBadge({ tier }: { tier: string }) {
  const paid = tier !== 'free'
  return (
    <span
      className={`flex-none rounded-sm border px-1.5 py-px text-[10px] uppercase tracking-wide ${
        paid ? 'border-koma-fg/40 text-koma-fg' : 'border-koma-border text-koma-dim'
      }`}
    >
      {tier || 'free'}
    </span>
  )
}

function InstallButton({
  id,
  installed,
  pending,
  onInstall,
  onUninstall,
}: {
  id: string
  installed: boolean
  pending: boolean
  onInstall: () => void
  onUninstall: () => void
}) {
  if (pending) {
    return (
      <span className="flex items-center gap-1.5 rounded-sm border border-koma-border px-2 py-1 text-[11px] text-koma-dim">
        <BrailleSpinner size={11} />working
      </span>
    )
  }
  if (installed) {
    return (
      <button
        onClick={onUninstall}
        className="flex items-center gap-1.5 rounded-sm border border-koma-border px-2 py-1 text-[11px] text-koma-dim transition hover:border-koma-fg/40 hover:text-koma-fg"
        title="Uninstall"
      >
        Uninstall
      </button>
    )
  }
  return (
    <button
      onClick={onInstall}
      className="flex items-center gap-1.5 rounded-sm border border-koma-fg/40 px-2 py-1 text-[11px] text-koma-fg transition hover:bg-koma-hover"
      title="Install"
    >
      Install
    </button>
  )
}

function grantLabel(g: string): string {
  const labels: Record<string, string> = {
    'agents:read': 'Read agents and sub-agents',
    'agents:orchestrate': 'Orchestrate sub-agents',
  }
  return labels[g] ?? g
}

function contributeLines(detail: {
  tools: unknown[]
  models: unknown[]
  panels: unknown[]
  subAgents: unknown[]
}): string[] {
  const out: string[] = []
  const push = (n: number, one: string, many: string) => {
    if (n > 0) out.push(`${n} ${n === 1 ? one : many}`)
  }
  push(detail.models.length, 'model provider', 'model providers')
  push(detail.panels.length, 'panel', 'panels')
  push(detail.tools.length, 'tool', 'tools')
  push(detail.subAgents.length, 'sub-agent', 'sub-agents')
  return out
}

// ─── Installed Extension Detail Tab (Tab-B) ─────────────────────────────────
// Rendered inside TabbedMain when the active tab kind is 'installedExtension'.
// Reads the detail from the `store.installedDetail` slice (populated by the
// two-phase InstalledExtensionDetail push: local first, then online enrichment).
// Merges online store metadata for presentation while keeping local data
// authoritative for installed version, permissions, and contributions.

export default function InstalledExtensionTab({ extId }: { extId: string }) {
  const detail = useKoma((s) => s.store.installedDetail)
  const loading = useKoma((s) => s.store.installedDetailLoading)
  const error = useKoma((s) => s.store.installedDetailError)
  const installed = useKoma((s) => s.store.installed)
  const uninstallExtension = useKoma((s) => s.uninstallExtension)
  const installExtension = useKoma((s) => s.installExtension)
  const pendingOp = useKoma((s) => s.store.pendingOp)
  const opResult = useKoma((s) => s.store.opResult)
  const openInstalledExtensionTab = useKoma((s) => s.openInstalledExtensionTab)
  const clearStoreNotice = useKoma((s) => s.clearStoreNotice)

  // Fire the request on mount (and when extId changes).
  useEffect(() => {
    openInstalledExtensionTab(extId)
  }, [extId])
  // eslint-disable-next-line react-hooks/exhaustive-deps

  const isInstalled = installed.some((e) => e.id === extId)

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center text-koma-dim">
        <BrailleSpinner size={18} className="opacity-70" />
      </div>
    )
  }

  if (error && !detail) {
    return (
      <div className="flex h-full items-center justify-center p-6 text-[13px] text-koma-error">
        {error}
      </div>
    )
  }

  if (!detail) return null

  // Online enrichment — merged for presentation, local data stays authoritative.
  const sd = detail.storeDetail
  const displayName = sd?.name || detail.name || detail.id
  const displayTagline = sd?.tagline || detail.description
  const displayAuthor = sd?.author
  const displayDescriptionMd = sd?.descriptionMd || detail.description
  const displayVersions = sd?.versions && sd.versions.length > 0 ? sd.versions : [detail.version]
  const latestVersion = sd?.latestVersion
  const hasUpdate = !!latestVersion && latestVersion !== detail.version

  const provides = contributeLines(detail)

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-5 p-5">
      {/* Header */}
      <div className="flex items-start gap-3">
        <div className="mt-0.5 flex-none text-koma-dim">
          {detail.kind === 'daemon' ? <Package size={26} /> : <Blocks size={26} />}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h1 className="truncate text-[18px] font-semibold text-koma-fg">{displayName}</h1>
            <TierBadge tier={detail.tier} />
          </div>
          {displayTagline && (
            <p className="mt-1 text-[13px] text-koma-dim">{displayTagline}</p>
          )}
          <p className="mt-0.5 text-[11px] text-koma-dim opacity-70">
            {displayAuthor ? `${displayAuthor} · ` : ''}v{detail.version} · {detail.id}
            {hasUpdate && (
              <span className="ml-2 text-koma-accent">
                (latest: v{latestVersion})
              </span>
            )}
          </p>
        </div>
        <div className="flex-none">
          {isInstalled ? (
            <InstallButton
              id={detail.id}
              installed
              pending={pendingOp === detail.id}
              onInstall={() => {}}
              onUninstall={() => uninstallExtension(detail.id)}
            />
          ) : (
            <InstallButton
              id={detail.id}
              installed={false}
              pending={pendingOp === detail.id}
              onInstall={() => installExtension(detail.id)}
              onUninstall={() => {}}
            />
          )}
        </div>
      </div>

      {/* Op result notice */}
      {opResult && (
        <div
          className={`rounded-md border px-3 py-2 text-[12px] ${
            opResult.ok
              ? 'border-koma-success/40 text-koma-success'
              : 'border-koma-error/40 text-koma-error'
          }`}
        >
          {opResult.message}
          <button
            onClick={clearStoreNotice}
            className="ml-2 text-koma-dim transition hover:text-koma-fg"
          >
            dismiss
          </button>
        </div>
      )}

      {/* Provides / Wants — local contribution counts and manifest requirements */}
      <div className="rounded-md border border-koma-border bg-koma-panel p-4 text-[12px]">
        <div className="flex flex-col gap-3 sm:flex-row sm:gap-8">
          <div className="min-w-0 flex-1">
            <div className="mb-1 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">
              Provides
            </div>
            {provides.length === 0 ? (
              <div className="text-koma-dim opacity-70">nothing yet</div>
            ) : (
              <ul className="flex flex-col gap-0.5 text-koma-fg">
                {provides.map((p) => (
                  <li key={p}>· {p}</li>
                ))}
              </ul>
            )}
          </div>
          <div className="min-w-0 flex-1">
            <div className="mb-1 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">
              Wants
            </div>
            {detail.requires.length === 0 ? (
              <div className="text-koma-dim opacity-70">nothing — self-contained</div>
            ) : (
              <ul className="flex flex-col gap-0.5 text-koma-fg">
                {detail.requires.map((r) => (
                  <li key={r}>· {grantLabel(r)}</li>
                ))}
              </ul>
            )}
          </div>
        </div>
      </div>

      {/* About — online markdown description or local description */}
      {displayDescriptionMd.trim() && (
        <div>
          <div className="mb-1 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">
            About
          </div>
          <pre className="whitespace-pre-wrap font-sans text-[13px] leading-relaxed text-koma-dim">
            {displayDescriptionMd}
          </pre>
        </div>
      )}

      {/* Versions */}
      {displayVersions.length > 0 && (
        <div className="text-[11px] text-koma-dim opacity-70">
          Versions: {displayVersions.join(', ')}
        </div>
      )}

      {/* Granted — local-only installed section */}
      {detail.granted.length > 0 && (
        <div className="rounded-md border border-koma-border bg-koma-panel p-4 text-[12px]">
          <div className="mb-1 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">
            Granted
          </div>
          <ul className="flex flex-col gap-0.5 text-koma-fg">
            {detail.granted.map((g) => (
              <li key={g}>· {grantLabel(g)}</li>
            ))}
          </ul>
        </div>
      )}

      {/* Tools — local-only installed section */}
      {detail.tools.length > 0 && (
        <section>
          <div className="mb-2 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">
            Tools
          </div>
          <ul className="flex flex-col gap-1">
            {detail.tools.map((t) => (
              <li
                key={t.name}
                className="rounded-md border border-koma-border bg-koma-panel px-3 py-2 text-[12px]"
              >
                <span className="font-semibold text-koma-fg">{t.name}</span>
                {t.description && (
                  <span className="ml-2 text-koma-dim">{t.description}</span>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}

      {/* Models — local-only installed section */}
      {detail.models.length > 0 && (
        <section>
          <div className="mb-2 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">
            Models
          </div>
          <ul className="flex flex-col gap-1">
            {detail.models.map((m) => (
              <li
                key={m.id}
                className="rounded-md border border-koma-border bg-koma-panel px-3 py-2 text-[12px]"
              >
                <span className="font-semibold text-koma-fg">{m.displayName}</span>
                <span className="ml-2 text-koma-dim opacity-70">{m.id}</span>
              </li>
            ))}
          </ul>
        </section>
      )}

      {/* Sub-agents — local-only installed section */}
      {detail.subAgents.length > 0 && (
        <section>
          <div className="mb-2 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">
            Sub-agents
          </div>
          <ul className="flex flex-col gap-1">
            {detail.subAgents.map((a) => (
              <li
                key={a.name}
                className="rounded-md border border-koma-border bg-koma-panel px-3 py-2 text-[12px]"
              >
                <span className="font-semibold text-koma-fg">{a.name}</span>
                {a.description && (
                  <span className="ml-2 text-koma-dim">{a.description}</span>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  )
}
