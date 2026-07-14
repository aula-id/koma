// A remote branch's short name for a checkout arg — strip the leading
// `<remote>/` segment (e.g. "origin/feature-x" -> "feature-x") so `git
// checkout <shortname>` DWIMs into a fresh LOCAL tracking branch instead of
// checking out the remote-tracking ref itself (which detaches HEAD). Shared
// by BranchSwitcher's remote branch rows and GraphContextMenu's remote ref
// chip checkout — single source of truth so neither can silently detach.
export function remoteShortName(name: string): string {
  const idx = name.indexOf('/')
  return idx >= 0 ? name.slice(idx + 1) : name
}
