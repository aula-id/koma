// ─── extension STORE (koma.run marketplace) ────────────────────────────────
// One catalogue row (host `StoreItemWire`, camelCase on the wire). The store
// browse/detail come from the PUBLIC koma.run endpoints; there are no secrets.
export type StoreItem = {
  id: string
  name: string
  tagline: string
  // 'free' | 'paid' — bare string so a future tier degrades gracefully.
  tier: string
  // 'daemon' | 'oneshot'.
  kind: string
  latestVersion: string
  iconUrl: string
  categories: string[]
  author: string
  updatedAt: string
}

// Per-kind contribution COUNTS (host `StoreContributesWire`) — the detail
// install card's "provides" summary.
export type StoreContributes = {
  models: number
  panels: number
  tools: number
  subAgents: number
}

// One extension's full detail (host `StoreDetailWire`) — the browse card plus
// the long description, screenshots, contribution counts, `requires` grant list
// (the install card's "wants" line), and available versions.
export type StoreDetail = StoreItem & {
  descriptionMd: string
  screenshots: string[]
  contributes: StoreContributes
  requires: string[]
  versions: string[]
}

// One contributed panel (host `PanelWire`, an installed extension's
// `contributes.panels[]` entry) — the activity-bar icon + tab framing for
// `koma://extension/<id>/...`.
export type ExtPanel = {
  id: string
  title: string
  icon: string
}

// One locally-installed extension (host `InstalledExtWire`). No tokens exist in
// the registry, so this is a full projection.
export type InstalledExt = {
  id: string
  /** Human-readable name from the installed manifest, when available. */
  name: string
  version: string
  tier: string
  kind: string
  enabled: boolean
  granted: string[]
  panels: ExtPanel[]
  /** Declared `workspace_dir` (data directory), when the manifest declares one —
   *  named in the uninstall confirm as the directory the nuke deletes. */
  workspaceDir?: string
}

// Full detail of one locally-installed extension — registry fields PLUS on-disk
// manifest contributions (tools/models/panels/sub-agents). The reply to
// `GetInstalledExtensionDetail`.
export type InstalledExtDetail = {
  id: string
  name: string
  version: string
  description: string
  tier: string
  kind: string
  enabled: boolean
  granted: string[]
  requires: string[]
  panels: ExtPanel[]
  tools: { name: string; description: string }[]
  models: { id: string; displayName: string }[]
  subAgents: { name: string; description: string }[]
  /** Best-effort online store enrichment — absent on the initial local response. */
  storeDetail?: StoreDetail | null
  /** Declared `workspace_dir` (data directory), when the manifest declares one —
   *  named in the uninstall confirm as the directory the nuke deletes. */
  workspaceDir?: string
}
