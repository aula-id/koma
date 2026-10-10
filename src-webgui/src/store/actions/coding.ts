import { backupCodingDocument, forgetCodingDraft, recordCodingHistory } from '../../lib/coding-recovery'
import { formatBeforeSave } from '../../lib/coding-save'
import { codingRequest } from '../../lib/coding-service'
import { queueReveal, uriToPath } from '../../lib/lsp-bridge'
import { localFileTabId } from '../../lib/composerChipOpen'
import { codingTabId, isMarkdownPath } from '../../lib/markdownPreview'
import type { StoreGet, StoreSet } from '../api'
import { baseName as codingBaseName, isPathOrDescendant as codingIsPathOrDescendant, emptyFileState, fileKey, mintRequestId } from '../coding'
import { insertGroup, normalizeGroups, reorderTab } from '../editorGroups'
import type { KomaState } from '../state'
import type { Tab } from '../types/tabs'

export function codingActions(set: StoreSet, get: StoreGet): Pick<KomaState, 'setBottomPanelTab' | 'refreshLsp' | 'lspInstall' | 'lspUninstall' | 'setProblemsOpen' | 'toggleProblemsOpen' | 'setLspDrawerOpen' | 'toggleLspDrawerOpen' | 'openDiagnostic' | 'openTerminalTab' | 'setActiveCodingRoot' | 'openCodingFile' | 'openLocalFileTab' | 'saveCodingFile' | 'revertCodingFile' | 'updateCodingContent' | 'createCodingItem' | 'renameCodingItem' | 'deleteCodingItem' | 'uploadCodingFile' | 'downloadCodingFile' | 'refreshCodingDir' | 'clearCodingConflict' | 'setCodingSearchQuery' | 'setCodingSearchReplace' | 'setCodingSearchFlag' | 'setCodingSearchGlobs' | 'searchCodingContent' | 'replaceCodingContentAll' | 'openCodingSearchHit'> {
  return {
  setBottomPanelTab: (tab) => set({ bottomPanelTab: tab }),
  refreshLsp: () => {
    get().req({ r: 'LspStatus' })
  },
  lspInstall: (id, all = false, force = false) => {
    get().req({
      r: 'LspInstall',
      id: all ? null : id,
      all,
      force,
    })
  },
  lspUninstall: (id) => {
    get().req({ r: 'LspUninstall', id })
  },
  setProblemsOpen: (open) =>
    set(s => ({ bottomPanelTab: open ? 'problems' : s.bottomPanelTab === 'problems' ? null : s.bottomPanelTab })),
  toggleProblemsOpen: () =>
    set(s => ({ bottomPanelTab: s.bottomPanelTab === 'problems' ? null : 'problems' })),
  setLspDrawerOpen: (open) =>
    set(s => ({ bottomPanelTab: open ? 'lsp' : s.bottomPanelTab === 'lsp' ? null : s.bottomPanelTab })),
  toggleLspDrawerOpen: () =>
    set(s => ({ bottomPanelTab: s.bottomPanelTab === 'lsp' ? null : 'lsp' })),
  openDiagnostic: (uri, line, character) => {
    const abs = uriToPath(uri)
    if (!abs) return
    const aa = abs.replace(/\\/g, '/')
    const roots = (get().settingsValues?.workdir ?? []).filter(Boolean)
    const candidates = [
      ...[...roots].sort((a, b) => b.length - a.length),
      ...get()
        .ui.tabs.filter((t): t is Extract<Tab, { kind: 'codingFile' }> => t.kind === 'codingFile')
        .map((t) => t.root),
      get().coding.activeRoot,
    ].filter((r): r is string => !!r)

    let root: string | null = null
    let rel = ''
    for (const r of candidates) {
      const rr = r.replace(/\\/g, '/').replace(/\/$/, '')
      if (aa === rr || aa.startsWith(rr + '/')) {
        root = r
        rel = aa === rr ? '' : aa.slice(rr.length + 1)
        break
      }
    }
    if (!root) return

    const targetLine = line + 1
    const targetCol = Math.max(1, character + 1)
    // Queue before open so a just-mounted tab can consume on first content paint.
    queueReveal(root, rel, targetLine, targetCol)
    get().openCodingFile(root, rel)
    const fire = () => {
      window.dispatchEvent(
        new CustomEvent('koma-reveal-line', {
          detail: { root, path: rel, line: targetLine, column: targetCol },
        }),
      )
    }
    // Event path for an already-mounted tab; queued reveal covers slow FileRead.
    fire()
    setTimeout(fire, 80)
    setTimeout(fire, 300)
  },
  openTerminalTab: (terminalId, title, shellId) => {
    const id = `term:${terminalId}`
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === id)
      const tab: Tab = { id, kind: 'terminal', terminalId, title }
      const tabs: Tab[] = exists ? s.ui.tabs : [...s.ui.tabs, tab]
      return { ui: { ...s.ui, tabs, activeTabId: id } }
    })
    // Send TerminalCreate to the host if this PTY hasn't been created yet.
    // When a session is attached, pass its primary workdir so remote shells
    // open there (local shells use it as the PTY cwd too).
    const created = (globalThis as any).__terminalsCreated ?? new Set<string>()
    if (!created.has(terminalId)) {
      created.add(terminalId)
      ;(globalThis as any).__terminalsCreated = created
      const cwd = get().settingsValues?.workdir?.[0] ?? undefined
      get().req({ r: 'TerminalCreate', id: terminalId, cwd, shell_id: shellId })
    }
  },
  setActiveCodingRoot: (root, opts) => {
    if (opts?.closeTabs) get().closeAllTabsExceptChat({ force: true })
    set((s) => ({ coding: { ...s.coding, activeRoot: root } }))
  },
  openLocalFileTab: (absPath, title) => {
    const id = localFileTabId(absPath)
    set((s) => {
      const baseUi = normalizeGroups(s.ui)
      const exists = baseUi.tabs.some((t) => t.id === id)
      const tabs: Tab[] = exists
        ? baseUi.tabs
        : [...baseUi.tabs, { id, kind: 'localFile', absPath, title }]
      return {
        ui: {
          ...baseUi,
          tabs,
          activeTabId: id,
          groupActive: { ...baseUi.groupActive, [baseUi.activeGroupId]: id },
        },
      }
    })
  },
  openCodingFile: (root, path, opts) => {
    const preview = !!opts?.preview && isMarkdownPath(path)
    const id = codingTabId(root, path, preview)
    const key = fileKey(root, path)
    const force = !!opts?.force
    // Reuse an already-loaded buffer for activate/split/move. Forcing a FileRead
    // + loading:true on every open remounted Monaco against a thrashing buffer
    // (setValue ↔ updateCodingContent → React #185) during edge-drop splits.
    const cached = get().coding.files[key]
    if (force && cached?.saving) return
    const reuseBuffer = !force && cached?.content != null && !cached.loading
    const requestId = reuseBuffer ? null : mintRequestId()
    set((s) => {
      const baseUi = normalizeGroups(s.ui)
      const exists = baseUi.tabs.some((t) => t.id === id)
      let tabs: Tab[] = exists
        ? baseUi.tabs
        : [...baseUi.tabs, { id, kind: 'codingFile', root, path, title: codingBaseName(path), preview }]
      let ui = { ...baseUi, tabs, activeTabId: id }

      const targetGroup =
        opts?.groupId && ui.groups.includes(opts.groupId) ? opts.groupId : ui.activeGroupId

      if (opts?.split) {
        // Edge drop: open the file into a new adjacent pane (or the target
        // group when the live pane count is already at MAX_GROUPS).
        const inserted = insertGroup(ui, targetGroup, opts.split.side, opts.split.dir)
        if (inserted) {
          ui = {
            ...ui,
            splitTree: inserted.splitTree,
            groups: inserted.groups,
            tabGroup: { ...ui.tabGroup, [id]: inserted.id },
            groupActive: { ...ui.groupActive, [inserted.id]: id },
            activeGroupId: inserted.id,
            activeTabId: id,
          }
        } else {
          tabs = reorderTab(tabs, id, null)
          ui = {
            ...ui,
            tabs,
            tabGroup: { ...ui.tabGroup, [id]: targetGroup },
            groupActive: { ...ui.groupActive, [targetGroup]: id },
            activeGroupId: targetGroup,
            activeTabId: id,
          }
        }
      } else if (opts?.groupId) {
        // Center / tab-strip drop: land in that group (optionally before a tab).
        const before =
          opts.beforeId != null && ui.tabGroup[opts.beforeId] === targetGroup
            ? opts.beforeId
            : null
        tabs = reorderTab(tabs, id, before)
        ui = {
          ...ui,
          tabs,
          tabGroup: { ...ui.tabGroup, [id]: targetGroup },
          groupActive: { ...ui.groupActive, [targetGroup]: id },
          activeGroupId: targetGroup,
          activeTabId: id,
        }
      } else if (exists) {
        // Plain activate: keep the tab in its current group, just focus it.
        const gid = ui.tabGroup[id] ?? ui.activeGroupId
        ui = {
          ...ui,
          groupActive: { ...ui.groupActive, [gid]: id },
          activeGroupId: gid,
          activeTabId: id,
        }
      }

      if (reuseBuffer) {
        return { ui: normalizeGroups(ui) }
      }

      const prev = s.coding.files[key]
      // force (Revert): clear dirty/conflict so FileRead may replace the buffer.
      // Without this, reduceFileRead keeps local edits and Revert is a no-op.
      const nextFile = force
        ? emptyFileState({
            content: prev?.savedContent ?? null,
            savedContent: prev?.savedContent ?? null,
            fingerprint: prev?.fingerprint ?? '',
            dirty: false,
            conflict: false,
            loading: true,
          })
        : { ...(prev ?? emptyFileState()), loading: true }
      return {
        ui: normalizeGroups(ui),
        coding: {
          ...s.coding,
          _readReq: { ...s.coding._readReq, [key]: requestId! },
          files: {
            ...s.coding.files,
            [key]: nextFile,
          },
        },
      }
    })
    if (requestId) get().req({ r: 'FileRead', root, path, requestId })
    if (opts?.groupId || opts?.split) get().syncStreamView()
  },
  saveCodingFile: async (root, path) => {
    const key = fileKey(root, path)
    let file = get().coding.files[key]
    if (!file || file.content == null || file.loading || file.conflict || file.binary || file.tooLarge) return
    if (file.saving) {
      set((s) => ({ coding: { ...s.coding, files: { ...s.coding.files, [key]: { ...file, saveQueued: true } } } }))
      return
    }
    if (!file.dirty) return
    const before = file
    const hostId = get().remoteState.hostId ?? 'local'
    const generation = get().coding._sessionGen
    try {
      const formatted = await formatBeforeSave({ hostId, root }, path, file.content!)
      if ((get().remoteState.hostId ?? 'local') !== hostId || get().coding._sessionGen !== generation || get().coding.files[key] !== before) return
      if (formatted !== file.content) get().updateCodingContent(root, path, formatted)
      file = get().coding.files[key]
    } catch (error) {
      if ((get().remoteState.hostId ?? 'local') === hostId && get().coding.files[key] === before) {
        const text = `Save canceled: ${error instanceof Error ? error.message : String(error)}`
        set(s => { const id = s.ui.toastSeq + 1; return { ui: { ...s.ui, toastSeq: id, toast: { id, text, kind: 'error' } } } })
      }
      return
    }
    const requestId = mintRequestId()
    set((s) => ({
      coding: {
        ...s.coding,
        files: { ...s.coding.files, [key]: { ...file, saving: true, pendingSave: { requestId, content: file.content! }, saveQueued: false, error: null } },
      },
    }))
    get().req({
      r: 'FileSave',
      root,
      path,
      content: file.content!,
      expectedFingerprint: file.fingerprint,
      requestId,
    })
  },
  revertCodingFile: (root, path) => {
    if (get().coding.files[fileKey(root, path)]?.saving) return
    const current = get().coding.files[fileKey(root, path)]
    const workspace = { hostId: get().remoteState.hostId ?? 'local', root }
    if (current?.content != null && current.dirty) recordCodingHistory(workspace, path, current.content, 'Before revert')
    forgetCodingDraft(workspace, path)
    get().openCodingFile(root, path, { force: true })
  },
  updateCodingContent: (root, path, content) => {
    const key = fileKey(root, path)
    // Pin LF so Monaco CRLF never becomes a distinct store write.
    const normalized = content.replace(/\r\n/g, '\n').replace(/\r/g, '\n')
    set((s) => {
      const prev = s.coding.files[key]
      if (!prev) return s
      const dirty = normalized !== (prev.savedContent ?? '')
      if (prev.content === normalized && prev.dirty === dirty) return s
      return {
        coding: {
          ...s.coding,
          files: {
            ...s.coding.files,
            [key]: { ...prev, content: normalized, dirty },
          },
        },
      }
    })
    const latest = get().coding.files[key]
    const workspace = { hostId: get().remoteState.hostId ?? 'local', root }
    if (latest?.dirty && latest.content != null) backupCodingDocument(workspace, path, {
      content: latest.content, savedContent: latest.savedContent, fingerprint: latest.fingerprint,
    })
    else if (latest && !latest.saving) forgetCodingDraft(workspace, path)
  },
  createCodingItem: (root, path, kind) => {
    get().req({ r: 'FileCreate', root, path, kind, requestId: mintRequestId() })
  },
  renameCodingItem: (root, oldPath, newPath) => {
    if (Object.entries(get().coding.files).some(([key, file]) => file.saving && key.startsWith(root + ':') && (codingIsPathOrDescendant(key.slice(root.length + 1), oldPath) || codingIsPathOrDescendant(key.slice(root.length + 1), newPath)))) {
      set(s => { const id = s.ui.toastSeq + 1; return { ui: { ...s.ui, toastSeq: id, toast: { id, text: 'Wait for the file to finish saving before renaming', kind: 'error' } } } })
      return
    }
    get().req({ r: 'FileRename', root, oldPath, newPath, requestId: mintRequestId() })
  },
  deleteCodingItem: (root, path) => {
    if (Object.entries(get().coding.files).some(([key, file]) => file.saving && key.startsWith(root + ':') && codingIsPathOrDescendant(key.slice(root.length + 1), path))) {
      set(s => { const id = s.ui.toastSeq + 1; return { ui: { ...s.ui, toastSeq: id, toast: { id, text: 'Wait for the file to finish saving before deleting', kind: 'error' } } } })
      return
    }
    get().req({ r: 'FileDelete', root, path, requestId: mintRequestId() })
  },
  uploadCodingFile: async (root, dirPath, file, overwrite = true) => {
    const name = (file.name || 'upload').replace(/^.*[/\\]/, '')
    if (!name || name.includes('..')) {
      const text = 'invalid file name'
      set((s) => {
        const seq = s.ui.toastSeq + 1
        return {
          ui: { ...s.ui, toastSeq: seq, toast: { id: seq, text, kind: 'error' } },
        }
      })
      return
    }
    // Match host FILE_BYTES_SIZE_CAP (25 MiB) before shipping base64 over IPC.
    const MAX = 25 * 1024 * 1024
    if (file.size > MAX) {
      const text = `file too large (max 25 MiB): ${name}`
      set((s) => {
        const seq = s.ui.toastSeq + 1
        return {
          ui: { ...s.ui, toastSeq: seq, toast: { id: seq, text, kind: 'error' } },
        }
      })
      return
    }
    const path = dirPath ? `${dirPath.replace(/\/+$/, '')}/${name}` : name
    try {
      const buf = await file.arrayBuffer()
      const bytes = new Uint8Array(buf)
      let binary = ''
      const chunk = 0x8000
      for (let i = 0; i < bytes.length; i += chunk) {
        binary += String.fromCharCode(...bytes.subarray(i, i + chunk))
      }
      const bytesB64 = btoa(binary)
      get().req({
        r: 'FileWriteBytes',
        root,
        path,
        bytesB64,
        overwrite: overwrite !== false,
        requestId: mintRequestId(),
      })
    } catch {
      const text = `failed to read file: ${name}`
      set((s) => {
        const seq = s.ui.toastSeq + 1
        return {
          ui: { ...s.ui, toastSeq: seq, toast: { id: seq, text, kind: 'error' } },
        }
      })
    }
  },
  downloadCodingFile: (root, path) => {
    // saveAs:true → host opens a native save dialog and writes the file.
    // wry cannot honor in-page blob downloads.
    get().req({
      r: 'FileDownloadBytes',
      root,
      path,
      requestId: mintRequestId(),
      saveAs: true,
    })
  },
  refreshCodingDir: (root, path) => {
    const key = fileKey(root, path)
    const requestId = mintRequestId()
    set((s) => ({
      coding: {
        ...s.coding,
        _treeReq: { ...s.coding._treeReq, [key]: requestId },
        dirs: {
          ...s.coding.dirs,
          [key]: { entries: s.coding.dirs[key]?.entries ?? [], loading: true, error: null },
        },
      },
    }))
    get().req({ r: 'FileTree', root, path, requestId })
  },
  clearCodingConflict: (root, path) => {
    const key = fileKey(root, path)
    set((s) => {
      const prev = s.coding.files[key]
      if (!prev) return s
      return {
        coding: {
          ...s.coding,
          files: { ...s.coding.files, [key]: { ...prev, conflict: false, error: null } },
        },
      }
    })
  },
  setCodingSearchQuery: (query) => {
    set((s) => ({
      coding: { ...s.coding, search: { ...s.coding.search, query, lastReplaceSummary: null } },
    }))
  },
  setCodingSearchReplace: (replace) => {
    set((s) => ({
      coding: { ...s.coding, search: { ...s.coding.search, replace } },
    }))
  },
  setCodingSearchFlag: (flag, value) => {
    set((s) => ({
      coding: {
        ...s.coding,
        search: { ...s.coding.search, [flag]: value, lastReplaceSummary: null },
      },
    }))
  },
  setCodingSearchGlobs: (includeGlob, excludeGlob) => {
    set((s) => ({
      coding: {
        ...s.coding,
        search: { ...s.coding.search, includeGlob, excludeGlob, lastReplaceSummary: null },
      },
    }))
  },
  searchCodingContent: (root) => {
    const search = get().coding.search
    const query = search.query
    if (!root || !query.trim()) {
      set((s) => ({
        coding: {
          ...s.coding,
          search: {
            ...s.coding.search,
            loading: false,
            error: null,
            results: [],
            truncated: false,
            _searchReq: null,
          },
        },
      }))
      return
    }
    const requestId = mintRequestId()
    set((s) => ({
      coding: {
        ...s.coding,
        search: {
          ...s.coding.search,
          loading: true,
          error: null,
          lastReplaceSummary: null,
          _searchReq: requestId,
        },
      },
    }))
    get().req({
      r: 'FileContentSearch',
      root,
      path: '',
      query,
      caseSensitive: search.caseSensitive,
      wholeWord: search.wholeWord,
      isRegex: search.isRegex,
      includeGlob: search.includeGlob.trim() || null,
      excludeGlob: search.excludeGlob.trim() || null,
      requestId,
    })
  },
  replaceCodingContentAll: (root) => {
    const state = get(), search = state.coding.search
    if (!root || !search.query.trim() || search.replacing) return
    const requestId = mintRequestId()
    const workspace = { hostId: state.remoteState.hostId ?? 'local', root }
    const generation = state.coding._sessionGen
    const snapshot = state.coding.files
    set(s => ({ coding: { ...s.coding, search: { ...s.coding.search, replacing: true, replaceError: null, lastReplaceSummary: null, _replaceReq: requestId } } }))
    void codingRequest<{ files: Array<{ path: string; before: string; after: string; fingerprint: string }>; matchCount: number; skipped: number }>(workspace, { op: 'replacePreview', options: {
      query: search.query, replacement: search.replace, caseSensitive: search.caseSensitive, wholeWord: search.wholeWord, isRegex: search.isRegex,
      includeGlob: search.includeGlob.trim() || null, excludeGlob: search.excludeGlob.trim() || null,
    } }).then(async result => {
      const { showCodingRefactor } = await import('../../components/CodingRefactor')
      if (get().coding.search._replaceReq !== requestId || get().coding._sessionGen !== generation || (get().remoteState.hostId ?? 'local') !== workspace.hostId) return
      const files = result.files.map(file => {
        const initial = snapshot[fileKey(root, file.path)]
        if (initial && (initial.dirty || initial.saving || initial.loading || initial.conflict || initial.content !== file.before)) throw new Error(`Save or resolve ${file.path} before replacing its disk contents`)
        return { ...file, initial, savedContent: file.before }
      })
      set(s => ({ coding: { ...s.coding, search: { ...s.coding.search, replacing: false, lastReplaceSummary: `${files.length} files ready for preview${result.skipped ? `; ${result.skipped} binary/large files skipped` : ''}` } } }))
      if (files.length) showCodingRefactor({ workspace, path: files[0].path, position: { line: 0, character: 0 }, mode: 'prepared', label: 'Replace All', snapshot, generation, staged: { workspace, files, label: 'Replace All', generation } })
    }).catch(error => {
      if (get().coding.search._replaceReq === requestId) set(s => ({ coding: { ...s.coding, search: { ...s.coding.search, replacing: false, replaceError: error instanceof Error ? error.message : String(error) } } }))
    })
  },
  openCodingSearchHit: (root, path, line, col = 1) => {
    const targetLine = Math.max(1, line)
    const targetCol = Math.max(1, col)
    queueReveal(root, path, targetLine, targetCol)
    get().openCodingFile(root, path)
    const fire = () => {
      window.dispatchEvent(
        new CustomEvent('koma-reveal-line', {
          detail: { root, path, line: targetLine, column: targetCol },
        }),
      )
    }
    fire()
    setTimeout(fire, 80)
    setTimeout(fire, 300)
  },
  }
}
