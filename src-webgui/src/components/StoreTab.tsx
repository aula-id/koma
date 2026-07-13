import { useEffect, useState, type FormEvent } from 'react'
import { ArrowLeft, Download, Trash2, Check, RefreshCw, Search, Package, Blocks, AlertCircle, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import type { StoreItem, StoreDetail } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'

function grantLabel(g: string): string {
  switch (g) {
    case 'agents:read': return 'watch your sub-agents'
    case 'agents:orchestrate': return 'run & steer your sub-agents'
    default: return g
  }
}

function contributeLines(c: StoreDetail['contributes']): string[] {
  const out: string[] = []
  const push = (n: number, one: string, many: string) => { if (n > 0) out.push(`${n} ${n === 1 ? one : many}`) }
  push(c.models, 'model provider', 'model providers')
  push(c.panels, 'panel', 'panels')
  push(c.tools, 'tool', 'tools')
  push(c.subAgents, 'sub-agent', 'sub-agents')
  return out
}

function TierBadge({ tier }: { tier: string }) {
  const paid = tier !== 'free'
  return <span className={`flex-none rounded-sm border px-1.5 py-px text-[10px] uppercase tracking-wide ${paid ? 'border-koma-fg/40 text-koma-fg' : 'border-koma-border text-koma-dim'}`}>{tier || 'free'}</span>
}

function StoreNotice({ error, opResult, onDismiss }: { error: string | null; opResult: { ok: boolean; message: string } | null; onDismiss: () => void }) {
  if (!error && !opResult) return null
  const isError = !!error || opResult?.ok === false
  const message = error ?? opResult?.message ?? ''
  return (
    <div className={`flex items-start gap-2 rounded-md border px-3 py-2 text-[12px] ${isError ? 'border-koma-error/40 bg-koma-error/10 text-koma-error' : 'border-koma-success/40 bg-koma-success/10 text-koma-success'}`}>
      {isError ? <AlertCircle size={14} className="mt-px flex-none" /> : <Check size={14} className="mt-px flex-none" />}
      <span className="min-w-0 flex-1">{message}</span>
      <button onClick={onDismiss} aria-label="Dismiss" className="flex-none opacity-60 transition hover:opacity-100"><X size={13} /></button>
    </div>
  )
}

function InstallButton({ id, installed, pending, onInstall, onUninstall }: { id: string; installed: boolean; pending: boolean; onInstall: () => void; onUninstall: () => void }) {
  if (pending) return <span className="flex items-center gap-1.5 rounded-sm border border-koma-border px-2 py-1 text-[11px] text-koma-dim"><BrailleSpinner size={11} />working</span>
  if (installed) return (
    <button onClick={(e) => { e.stopPropagation(); onUninstall() }} className="group/btn flex items-center gap-1.5 rounded-sm border border-koma-border px-2 py-1 text-[11px] text-koma-dim transition hover:border-koma-fg/40 hover:text-koma-fg" title="Uninstall">
      <Check size={12} className="group-hover/btn:hidden" /><Trash2 size={12} className="hidden group-hover/btn:block" /><span className="group-hover/btn:hidden">Installed</span><span className="hidden group-hover/btn:block">Uninstall</span>
    </button>
  )
  return <button onClick={(e) => { e.stopPropagation(); onInstall() }} className="flex items-center gap-1.5 rounded-sm border border-koma-fg/40 px-2 py-1 text-[11px] text-koma-fg transition hover:bg-koma-hover" title="Install"><Download size={12} />Install</button>
}

function StoreCard({ item, installed, pending, onOpen, onInstall, onUninstall }: { item: StoreItem; installed: boolean; pending: boolean; onOpen: () => void; onInstall: () => void; onUninstall: () => void }) {
  return (
    <div role="button" tabIndex={0} onClick={onOpen} onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') onOpen() }} className="flex cursor-pointer flex-col gap-2 rounded-md border border-koma-border bg-koma-panel p-3 transition hover:border-koma-fg/30 hover:bg-koma-hover">
      <div className="flex items-start gap-2">
        <div className="mt-px flex-none text-koma-dim">{item.kind === 'daemon' ? <Package size={16} /> : <Blocks size={16} />}</div>
        <div className="min-w-0 flex-1"><div className="flex items-center gap-2"><span className="truncate text-[13px] font-semibold text-koma-fg">{item.name || item.id}</span><TierBadge tier={item.tier} /></div><div className="truncate text-[11px] text-koma-dim">{item.author}</div></div>
      </div>
      <p className="line-clamp-2 min-h-[2.4em] text-[12px] text-koma-dim">{item.tagline}</p>
      <div className="flex items-center justify-between">
        <div className="flex min-w-0 flex-wrap gap-1">{item.categories.slice(0, 2).map((c) => <span key={c} className="rounded-sm bg-koma-hover px-1.5 py-px text-[10px] text-koma-dim">{c}</span>)}</div>
        <InstallButton id={item.id} installed={installed} pending={pending} onInstall={onInstall} onUninstall={onUninstall} />
      </div>
    </div>
  )
}

function DetailView({ detail, installed, pending, error, opResult, onBack, onInstall, onUninstall, onDismissNotice }: { detail: StoreDetail; installed: boolean; pending: boolean; error: string | null; opResult: { ok: boolean; message: string } | null; onBack: () => void; onInstall: () => void; onUninstall: () => void; onDismissNotice: () => void }) {
  const provides = contributeLines(detail.contributes)
  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-5 p-5">
      <button onClick={onBack} className="flex w-fit items-center gap-1.5 text-[12px] text-koma-dim transition hover:text-koma-fg"><ArrowLeft size={14} />Back to store</button>
      <StoreNotice error={error} opResult={opResult} onDismiss={onDismissNotice} />
      <div className="flex items-start gap-3">
        <div className="mt-0.5 flex-none text-koma-dim">{detail.kind === 'daemon' ? <Package size={26} /> : <Blocks size={26} />}</div>
        <div className="min-w-0 flex-1"><div className="flex items-center gap-2"><h1 className="truncate text-[18px] font-semibold text-koma-fg">{detail.name || detail.id}</h1><TierBadge tier={detail.tier} /></div><p className="text-[13px] text-koma-dim">{detail.tagline}</p><p className="mt-0.5 text-[11px] text-koma-dim opacity-70">{detail.author} · v{detail.latestVersion} · {detail.id}</p></div>
        <div className="flex-none"><InstallButton id={detail.id} installed={installed} pending={pending} onInstall={onInstall} onUninstall={onUninstall} /></div>
      </div>
      <div className="rounded-md border border-koma-border bg-koma-panel p-4 text-[12px]"><div className="flex flex-col gap-3 sm:flex-row sm:gap-8"><div className="min-w-0 flex-1"><div className="mb-1 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">Provides</div>{provides.length === 0 ? <div className="text-koma-dim opacity-70">nothing yet</div> : <ul className="flex flex-col gap-0.5 text-koma-fg">{provides.map((p) => <li key={p}>· {p}</li>)}</ul>}</div><div className="min-w-0 flex-1"><div className="mb-1 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">Wants</div>{detail.requires.length === 0 ? <div className="text-koma-dim opacity-70">nothing — self-contained</div> : <ul className="flex flex-col gap-0.5 text-koma-fg">{detail.requires.map((r) => <li key={r}>· {grantLabel(r)}</li>)}</ul>}</div></div></div>
      {detail.descriptionMd.trim() && <div><div className="mb-1 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">About</div><pre className="whitespace-pre-wrap font-sans text-[13px] leading-relaxed text-koma-dim">{detail.descriptionMd}</pre></div>}
      {detail.versions.length > 0 && <div className="text-[11px] text-koma-dim opacity-70">Versions: {detail.versions.join(', ')}</div>}
    </div>
  )
}

export default function StoreTab() {
  const catalogue = useKoma((s) => s.store.catalogue)
  const detail = useKoma((s) => s.store.detail)
  const installed = useKoma((s) => s.store.installed)
  const busy = useKoma((s) => s.store.busy)
  const error = useKoma((s) => s.store.error)
  const opResult = useKoma((s) => s.store.opResult)
  const pendingOp = useKoma((s) => s.store.pendingOp)
  const browseStore = useKoma((s) => s.browseStore)
  const openStoreDetail = useKoma((s) => s.openStoreDetail)
  const closeStoreDetail = useKoma((s) => s.closeStoreDetail)
  const installExtension = useKoma((s) => s.installExtension)
  const uninstallExtension = useKoma((s) => s.uninstallExtension)
  const refreshInstalled = useKoma((s) => s.refreshInstalled)
  const clearStoreNotice = useKoma((s) => s.clearStoreNotice)
  const openInstalledExtensionTab = useKoma((s) => s.openInstalledExtensionTab)
  const [query, setQuery] = useState('')

  useEffect(() => { browseStore(); refreshInstalled() }, [])

  const installedIds = new Set(installed.map((e) => e.id))
  const onSearch = (e: FormEvent) => { e.preventDefault(); browseStore(query.trim() || undefined) }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-none items-center gap-3 border-b border-koma-border px-4 py-3">
        <Blocks size={16} className="flex-none text-koma-fg" />
        <h1 className="flex-none text-[13px] font-semibold text-koma-fg">Extensions</h1>
        <form onSubmit={onSearch} className="relative ml-2 min-w-0 flex-1">
          <Search size={13} className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-koma-dim opacity-60" />
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search the store…" className="w-full rounded-sm border border-koma-border bg-koma-bg py-1 pl-7 pr-2 text-[12px] text-koma-fg placeholder:text-koma-dim placeholder:opacity-60 focus:border-koma-fg/40 focus:outline-none" />
        </form>
        <button onClick={() => browseStore(query.trim() || undefined)} disabled={busy} title="Refresh" aria-label="Refresh store" className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-dim transition hover:bg-koma-hover hover:text-koma-fg disabled:cursor-wait">{busy ? <BrailleSpinner size={13} /> : <RefreshCw size={13} />}</button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {detail !== null ? (
          <DetailView detail={detail} installed={installedIds.has(detail.id)} pending={pendingOp === detail.id} error={error} opResult={opResult} onBack={closeStoreDetail} onInstall={() => installExtension(detail.id)} onUninstall={() => uninstallExtension(detail.id)} onDismissNotice={clearStoreNotice} />
        ) : busy && catalogue.length === 0 ? (
          <div className="flex h-full items-center justify-center text-koma-dim"><BrailleSpinner size={18} className="opacity-70" /></div>
        ) : (
          <div className="flex flex-col gap-6 p-4">
            <StoreNotice error={error} opResult={opResult} onDismiss={clearStoreNotice} />
            {installed.length > 0 && (
              <section>
                <div className="mb-2 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">Installed ({installed.length})</div>
                <ul className="flex flex-col gap-1">{installed.map((ext) => (
                  <li key={ext.id} className="flex items-center gap-2 rounded-md border border-koma-border bg-koma-panel px-3 py-2">
                    {ext.kind === 'daemon' ? <Package size={14} className="flex-none text-koma-dim" /> : <Blocks size={14} className="flex-none text-koma-dim" />}
                    <button onClick={() => openInstalledExtensionTab(ext.id)} className="min-w-0 flex-1 truncate text-left text-[12px] text-koma-fg hover:underline" title={ext.id}>{ext.id}</button>
                    <span className="flex-none text-[10px] text-koma-dim opacity-70">v{ext.version}</span>
                    <TierBadge tier={ext.tier} />
                    <InstallButton id={ext.id} installed pending={pendingOp === ext.id} onInstall={() => installExtension(ext.id)} onUninstall={() => uninstallExtension(ext.id)} />
                  </li>
                ))}</ul>
              </section>
            )}
            <section>
              <div className="mb-2 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">Browse</div>
              {catalogue.length === 0 ? <div className="py-8 text-center text-[12px] text-koma-dim opacity-70">{error ? 'Could not reach the store.' : 'No extensions found.'}</div> : (
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">{catalogue.map((item) => <StoreCard key={item.id} item={item} installed={installedIds.has(item.id)} pending={pendingOp === item.id} onOpen={() => openStoreDetail(item.id)} onInstall={() => installExtension(item.id)} onUninstall={() => uninstallExtension(item.id)} />)}</div>
              )}
            </section>
          </div>
        )}
      </div>
    </div>
  )
}
