import { useEffect } from 'react'
import { Package, Blocks } from 'lucide-react'
import { BrailleSpinner } from './BrailleSpinner'
import { useKoma } from '../store/koma'

// Shared helpers imported from StoreTab for consistency.
function TierBadge({ tier }: { tier: string }) {
  return tier === 'paid' ? (
    <span className="rounded bg-koma-accent/15 px-1.5 py-0.5 text-[10px] font-semibold text-koma-accent">PRO</span>
  ) : null
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
  if (installed) {
    return (
      <button
        disabled={pending}
        onClick={onUninstall}
        className="rounded border border-koma-border bg-koma-panel px-3 py-1.5 text-[12px] text-koma-fg hover:bg-koma-hover disabled:opacity-50"
      >
        {pending ? 'Uninstalling…' : 'Uninstall'}
      </button>
    )
  }
  return (
    <button
      disabled={pending}
      onClick={onInstall}
      className="rounded bg-koma-accent/20 px-3 py-1.5 text-[12px] font-semibold text-koma-accent hover:bg-koma-accent/30 disabled:opacity-50"
    >
      {pending ? 'Installing…' : 'Install'}
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

// ─── Installed Extension Detail Tab (Tab-B) ─────────────────────────────────
// Rendered inside TabbedMain when the active tab kind is 'installedExtension'.
// Reads the detail from the `store.installedDetail` slice (populated by the
// InstalledExtensionDetail push); fires the request on mount if not yet loaded.
// Only requests the new local detail and calls uninstallExtension; no StoreDetail
// or marketplace dependency.

export default function InstalledExtensionTab({ extId }: { extId: string }) {
  const detail = useKoma((s) => s.store.installedDetail)
  const loading = useKoma((s) => s.store.installedDetailLoading)
  const error = useKoma((s) => s.store.installedDetailError)
  const installed = useKoma((s) => s.store.installed)
  const uninstallExtension = useKoma((s) => s.uninstallExtension)
  const pendingOp = useKoma((s) => s.store.pendingOp)
  const opResult = useKoma((s) => s.store.opResult)
  const openInstalledExtensionTab = useKoma((s) => s.openInstalledExtensionTab)

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

  const contributes: string[] = []
  if (detail.tools.length > 0) contributes.push(`${detail.tools.length} tool${detail.tools.length === 1 ? '' : 's'}`)
  if (detail.models.length > 0) contributes.push(`${detail.models.length} model${detail.models.length === 1 ? '' : 's'}`)
  if (detail.panels.length > 0) contributes.push(`${detail.panels.length} panel${detail.panels.length === 1 ? '' : 's'}`)
  if (detail.subAgents.length > 0) contributes.push(`${detail.subAgents.length} sub-agent${detail.subAgents.length === 1 ? '' : 's'}`)

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-5 p-5">
      <div className="flex items-start gap-3">
        <div className="mt-0.5 flex-none text-koma-dim">
          {detail.kind === 'daemon' ? <Package size={26} /> : <Blocks size={26} />}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h1 className="truncate text-[18px] font-semibold text-koma-fg">{detail.name}</h1>
            <TierBadge tier={detail.tier} />
          </div>
          {detail.description && (
            <p className="mt-1 text-[13px] text-koma-dim">{detail.description}</p>
          )}
          <p className="mt-0.5 text-[11px] text-koma-dim opacity-70">
            v{detail.version} · {detail.id}
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
            <span className="text-[11px] text-koma-dim">No longer installed</span>
          )}
        </div>
      </div>

      {opResult && (
        <div className={`rounded-md border px-3 py-2 text-[12px] ${opResult.ok ? 'border-koma-success/40 text-koma-success' : 'border-koma-error/40 text-koma-error'}`}>
          {opResult.message}
        </div>
      )}

      {/* Grants */}
      {detail.granted.length > 0 && (
        <div className="rounded-md border border-koma-border bg-koma-panel p-4 text-[12px]">
          <div className="mb-1 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">Granted</div>
          <ul className="flex flex-col gap-0.5 text-koma-fg">
            {detail.granted.map((g) => (
              <li key={g}>· {grantLabel(g)}</li>
            ))}
          </ul>
        </div>
      )}

      {/* Requires */}
      {detail.requires && detail.requires.length > 0 && (
        <div className="rounded-md border border-koma-border bg-koma-panel p-4 text-[12px]">
          <div className="mb-1 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">Requires</div>
          <ul className="flex flex-col gap-0.5 text-koma-fg">
            {detail.requires.map((r) => (
              <li key={r}>· {r}</li>
            ))}
          </ul>
        </div>
      )}

      {/* Contributions */}
      {contributes.length > 0 && (
        <div className="rounded-md border border-koma-border bg-koma-panel p-4 text-[12px]">
          <div className="mb-1 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">Provides</div>
          <ul className="flex flex-col gap-0.5 text-koma-fg">
            {contributes.map((c) => (
              <li key={c}>· {c}</li>
            ))}
          </ul>
        </div>
      )}

      {/* Tools detail */}
      {detail.tools.length > 0 && (
        <section>
          <div className="mb-2 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">Tools</div>
          <ul className="flex flex-col gap-1">
            {detail.tools.map((t) => (
              <li key={t.name} className="rounded-md border border-koma-border bg-koma-panel px-3 py-2 text-[12px]">
                <span className="font-semibold text-koma-fg">{t.name}</span>
                {t.description && (
                  <span className="ml-2 text-koma-dim">{t.description}</span>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}

      {/* Models detail */}
      {detail.models.length > 0 && (
        <section>
          <div className="mb-2 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">Models</div>
          <ul className="flex flex-col gap-1">
            {detail.models.map((m) => (
              <li key={m.id} className="rounded-md border border-koma-border bg-koma-panel px-3 py-2 text-[12px]">
                <span className="font-semibold text-koma-fg">{m.displayName}</span>
                <span className="ml-2 text-koma-dim opacity-70">{m.id}</span>
              </li>
            ))}
          </ul>
        </section>
      )}

      {/* Sub-agents detail */}
      {detail.subAgents.length > 0 && (
        <section>
          <div className="mb-2 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">Sub-agents</div>
          <ul className="flex flex-col gap-1">
            {detail.subAgents.map((a) => (
              <li key={a.name} className="rounded-md border border-koma-border bg-koma-panel px-3 py-2 text-[12px]">
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
