import { forgetCodingDraft } from '../../lib/coding-recovery'
import type { StoreGet, StoreSet } from '../api'
import { emptyFileState, fileKey } from '../coding'
import { dropDesignDocs, dropDesignFileUi } from '../design'
import { dropDiagramDocs } from '../diagram'
import { setSplitDir as applySplitDir, toggleSplitDir as flipSplitDir, insertGroup, neighbourInGroup, normalizeGroups, reorderTab, resizeSplit } from '../editorGroups'
import { tabBaseName } from '../initial'
import type { KomaState } from '../state'
import type { Tab } from '../types/tabs'

export function tabActions(set: StoreSet, get: StoreGet): Pick<KomaState, 'openDiffTab' | 'closeTab' | 'closeAllTabsExceptChat' | 'activateTab' | 'focusEditorGroup' | 'moveTabToGroup' | 'splitTab' | 'toggleSplitDir' | 'setSplitDir' | 'resizeEditorGroups'> {
  return {
  openDiffTab: (path) => {
    const id = `diff:${path}`
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === id)
      const tabs: Tab[] = exists
        ? s.ui.tabs.map((t) =>
            t.id === id && t.kind === 'diff' ? { ...t, loading: true } : t,
          )
        : [...s.ui.tabs, { id, kind: 'diff', path, title: tabBaseName(path), loading: true }]
      return { ui: { ...s.ui, tabs, activeTabId: id } }
    })
    get().req({ r: 'FileDiff', path })
  },
  closeTab: (id, opts) => {
    if (id === 'chat') return
    const gitTab = get().ui.tabs.find(t => t.id === id)
    if (gitTab?.kind === 'gitTool' && gitTab.dirty && !opts?.force) return
    // When closing a terminal tab, tell the host to kill the PTY.
    {
      const closing = get().ui.tabs.find((t) => t.id === id)
      if (closing && closing.kind === 'terminal') {
        get().req({ r: 'TerminalKill', id: closing.terminalId })
        const created = (globalThis as any).__terminalsCreated as Set<string> | undefined
        created?.delete(closing.terminalId)
      }
    }
    // Dirty codingFile tabs confirm before discard unless the caller already
    // collected an explicit force (floating dirty-close popover).
    const closingCoding = (() => {
      const closing = get().ui.tabs.find((t) => t.id === id)
      return closing && closing.kind === 'codingFile' && !closing.preview ? closing : null
    })()
    // A close/discard cannot cancel a write already accepted by the host.
    if (closingCoding && get().coding.files[fileKey(closingCoding.root, closingCoding.path)]?.saving) return
    if (closingCoding && !opts?.force) {
      const f = get().coding.files[fileKey(closingCoding.root, closingCoding.path)]
      if (f?.dirty) return
    }
    const closingDiagram = (() => {
      const closing = get().ui.tabs.find((t) => t.id === id)
      return closing && closing.kind === 'diagram' ? closing : null
    })()
    if (closingDiagram) {
      const doc = get().diagram.docs[fileKey(closingDiagram.root, closingDiagram.path)]
      if (doc?.saving) return
      if (doc?.dirty && !opts?.force) return
    }
    const closingDesign = (() => {
      const closing = get().ui.tabs.find((t) => t.id === id)
      return closing && closing.kind === 'design' ? closing : null
    })()
    if (closingDesign) {
      const doc = get().design?.docs?.[fileKey(closingDesign.root, closingDesign.path)]
      if (doc?.saving) return
      if (doc?.dirty && !opts?.force) return
    }
    if (closingCoding) {
      if (opts?.force) forgetCodingDraft({ hostId: get().remoteState.hostId ?? 'local', root: closingCoding.root }, closingCoding.path)
      get().req({ r: 'LspDidClose', root: closingCoding.root, path: closingCoding.path })
      // Close-without-save must drop the dirty buffer + Monaco model; otherwise
      // reopen restores unsaved edits (reduceFileRead refuses to clobber dirty).
      if (opts?.force) {
        const root = closingCoding.root
        const path = closingCoding.path
        void import('../../lib/monaco-lsp')
          .then((m) => m.disposeCodingModel(root, path))
          .catch(() => {
            /* monaco not loaded yet */
          })
      }
    }
    const closedAnalytics = id === 'analytics'
    set((s) => {
      const normalized = normalizeGroups(s.ui)
      const idx = normalized.tabs.findIndex((t) => t.id === id)
      if (idx < 0) return s
      const tabs = normalized.tabs.filter((t) => t.id !== id)
      // A split has multiple active tabs. If the closed tab owns focus, stay in
      // its pane by preferring that pane's left/right neighbour. If it was the
      // pane's last tab, normalizeGroups collapses the empty pane and selects a
      // surviving group's active tab.
      const activeTabId =
        normalized.activeTabId === id
          ? neighbourInGroup(normalized, id) ?? id
          : normalized.activeTabId

      let coding = s.coding
      if (closingCoding && opts?.force) {
        const key = fileKey(closingCoding.root, closingCoding.path)
        const { [key]: _dropped, ...files } = coding.files
        const { [key]: _req, ..._readReq } = coding._readReq
        // A surviving read-only preview must show the saved text after discard.
        // It shares the buffer, but never owns or discards the source's edits.
        if (_dropped && tabs.some((t) => t.kind === 'codingFile' && t.preview && t.root === closingCoding.root && t.path === closingCoding.path)) {
          files[key] = emptyFileState({
            content: _dropped.savedContent ?? '',
            savedContent: _dropped.savedContent,
            fingerprint: _dropped.fingerprint,
          })
        }
        coding = { ...coding, files, _readReq }
      }

      const diagram =
        closingDiagram && opts?.force
          ? { ...s.diagram, docs: dropDiagramDocs(s.diagram.docs, closingDiagram.root, closingDiagram.path) }
          : s.diagram
      let design = s.design
      if (closingDesign && opts?.force) {
        const fallbackPanel =
          s.design?.panelTabId === id
            ? tabs.find((t) => t.kind === 'design')?.id ?? null
            : s.design?.panelTabId ?? null
        design = {
          ...s.design,
          panelTabId: fallbackPanel,
          docs: dropDesignDocs(s.design?.docs, closingDesign.root, closingDesign.path),
          fileUi: dropDesignFileUi(s.design?.fileUi, closingDesign.root, closingDesign.path),
        }
      }
      return {
        ui: normalizeGroups({ ...normalized, tabs, activeTabId }),
        coding,
        diagram,
        design,
        // Closing the Analytics tab drops its in-flight state so a later reopen
        // starts clean (filters preserved as user preference; data cleared so a
        // stale session-scoped payload can't reappear).
        ...(closedAnalytics
          ? {
              analytics: {
                ...s.analytics,
                loading: false,
                error: null,
                data: null,
                hasData: false,
              },
            }
          : {}),
      }
    })
    // The active tab may have changed (closed the active one) — re-sync the stream
    // view so the host stops streaming a just-closed stream tab's target (or starts
    // streaming the neighbour if focus fell onto another stream tab).
    get().syncStreamView()
  },
  closeAllTabsExceptChat: (opts) => {
    const force = opts?.force ?? true
    const ids = get().ui.tabs.filter((t) => t.id !== 'chat').map((t) => t.id)
    for (const id of ids) get().closeTab(id, { force })
    get().syncStreamView()
  },
  activateTab: (id) => {
    const tab = get().ui.tabs.find((t) => t.id === id)
    if (!tab) return
    const isDiff = tab != null && tab.kind === 'diff'
    set((s) => {
      const normalized = normalizeGroups(s.ui)
      return {
        ui: normalizeGroups({
          ...normalized,
          activeTabId: id,
          // Mark a re-focused diff tab loading for the re-request below, but keep
          // its existing `diff` so the editor doesn't flash to a spinner.
          tabs: isDiff
            ? normalized.tabs.map((t) =>
                t.id === id && t.kind === 'diff' ? { ...t, loading: true } : t,
              )
            : normalized.tabs,
        }),
      }
    })
    // A GIT-panel diff tab (has `staged`) re-requests via GitDiff, echoing
    // the SAME staged/unstaged side it was opened for; a plain File-changed
    // diff tab (no `staged`) re-requests via FileDiff — the two paths are
    // NOT interchangeable (different host handlers, different tab-id scheme).
    if (isDiff && tab.kind === 'diff') {
      if (tab.commitSha !== undefined) {
        // A commit-graph diff tab re-requests via GitCommitDiff — checked FIRST
        // (it carries no `staged`, so it would otherwise wrongly fall into the
        // FileDiff branch and fetch a working-tree diff for a historical path).
        get().req({ r: 'GitCommitDiff', sha: tab.commitSha, path: tab.path })
      } else if (tab.staged !== undefined) {
        get().req({ r: 'GitDiff', path: tab.path, staged: tab.staged })
      } else {
        get().req({ r: 'FileDiff', path: tab.path })
      }
    }
    // Re-focusing the Settings tab re-requests its values so they're fresh (the
    // name/workdir may have changed via other paths, e.g. the RenameOverlay).
    // Also re-fetch the SSH Keys vault list — the Settings tab stays mounted
    // (CSS-hidden) across a close/reopen, so without this the "SSH Keys"
    // section would only ever reflect whatever the vault looked like on the
    // FIRST open of the session.
    if (tab.kind === 'settings') {
      get().req({ r: 'GetSettings' })
      get().refreshKeys()
    }
    // Re-focusing the Analytics tab re-requests so the ledger is fresh.
    if (tab.kind === 'analytics') {
      get().refreshAnalytics()
    }
    // Re-focusing the Store tab refreshes the installed registry (an extension
    // may have been installed/removed via another path). The catalogue is left
    // as-is (a browse is user-initiated) so re-focus doesn't re-hit the network.
    if (tab.kind === 'store') {
      get().refreshInstalled()
    }
    // Re-focusing an installed-extension detail tab refreshes its detail.
    if (tab.kind === 'installedExtension') {
      set((s) => ({ store: { ...s.store, installedDetailRequestId: tab.extId, installedDetailLoading: true, installedDetailError: null } }))
      get().req({ r: 'GetInstalledExtensionDetail', id: tab.extId })
    }
    // Sync the stream view to the now-active tab: a stream tab → stream its target;
    // any other tab (chat/diff/settings) → clear the view. The host/daemon dedupe an
    // unchanged view, so activating a non-stream tab repeatedly is cheap.
    get().syncStreamView()
  },
  focusEditorGroup: (groupId) => {
    set((s) => {
      const ui = normalizeGroups(s.ui)
      if (!ui.groups.includes(groupId)) return s
      const activeTabId = ui.groupActive[groupId]
      if (!activeTabId) return s
      if (ui.activeGroupId === groupId && ui.activeTabId === activeTabId) return s
      return {
        ui: normalizeGroups({ ...ui, activeGroupId: groupId, activeTabId }),
      }
    })
    get().syncStreamView()
  },
  moveTabToGroup: (tabId, groupId, beforeId = null) => {
    if (tabId === 'chat') return
    set((s) => {
      const ui = normalizeGroups(s.ui)
      if (!ui.groups.includes(groupId) || !ui.tabs.some((t) => t.id === tabId)) return s
      const before =
        beforeId != null && ui.tabGroup[beforeId] === groupId ? beforeId : null
      const tabs = reorderTab(ui.tabs, tabId, before)
      return {
        ui: normalizeGroups({
          ...ui,
          tabs,
          tabGroup: { ...ui.tabGroup, [tabId]: groupId },
          groupActive: { ...ui.groupActive, [groupId]: tabId },
          activeGroupId: groupId,
          activeTabId: tabId,
        }),
      }
    })
    get().syncStreamView()
  },
  splitTab: (tabId, targetGroupId, side, dir) => {
    if (tabId === 'chat') return
    set((s) => {
      const ui = normalizeGroups(s.ui)
      if (!ui.tabs.some((t) => t.id === tabId)) return s
      const inserted = insertGroup(ui, targetGroupId, side, dir)
      if (!inserted) return s
      return {
        ui: normalizeGroups({
          ...ui,
          splitTree: inserted.splitTree,
          groups: inserted.groups,
          tabGroup: { ...ui.tabGroup, [tabId]: inserted.id },
          groupActive: { ...ui.groupActive, [inserted.id]: tabId },
          activeGroupId: inserted.id,
          activeTabId: tabId,
        }),
      }
    })
    get().syncStreamView()
  },
  toggleSplitDir: (groupId) =>
    set((s) => {
      const ui = normalizeGroups(s.ui)
      const next = flipSplitDir(ui, groupId)
      if (!next) return s
      return { ui: normalizeGroups({ ...ui, splitTree: next.splitTree }) }
    }),
  setSplitDir: (dir, groupId) =>
    set((s) => {
      const ui = normalizeGroups(s.ui)
      const next = applySplitDir(ui, dir, groupId)
      if (!next) return s
      return { ui: normalizeGroups({ ...ui, splitTree: next.splitTree }) }
    }),
  resizeEditorGroups: (splitId, deltaPx, totalPx) =>
    set((s) => {
      const ui = normalizeGroups(s.ui)
      const tree = ui.splitTree
      if (!tree) return s
      const splitTree = resizeSplit(tree, splitId, deltaPx, totalPx)
      if (splitTree === tree) return s
      return { ui: { ...ui, splitTree } }
    }),
  }
}
