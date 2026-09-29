import { invalidateCodingConfig } from '../../lib/coding-config'
import { backupCodingDocument, forgetCodingDraft, recordCodingHistory } from '../../lib/coding-recovery'
import { codingRequest, codingWindowId, resolveCodingReply } from '../../lib/coding-service'
import { resolveFilePreviewBytes } from '../../lib/filePreview'
import { resolveLspCompletion, resolveLspCompletionResolve, resolveLspDefinition, resolveLspDocumentSymbol, resolveLspFileText, resolveLspHover, resolveLspReferences } from '../../lib/lsp-bridge'
import { diagramTabId } from '../../lib/diagram'
import { codingTabId } from '../../lib/markdownPreview'
import { beginDiagramSeed } from '../actions/diagram'
import type { StoreGet, StoreSet } from '../api'
import { baseName as codingBaseName, isPathOrDescendant as codingIsPathOrDescendant, remapPath as codingRemapPath, fileKey, reduceFileContentReplace, reduceFileContentSearch, reduceFileCreate, reduceFileDelete, reduceFileRead, reduceFileRename, reduceFileSave, reduceFileTree, reduceFileWriteBytes } from '../coding'
import { claimDiagramRead, claimDiagramSave, dropDiagramDocs, remapDiagramDocs } from '../diagram'
import { normalizeGroups } from '../editorGroups'
import type { PushEnvelope } from '../types/envelope'
import type { Tab } from '../types/tabs'

export function pushCoding(set: StoreSet, get: StoreGet, env: PushEnvelope): boolean {
  switch (env.k) {
      case 'CodingReply': resolveCodingReply(env); break
      case 'CodingEvent': {
        if (env.clientId && env.clientId !== codingWindowId) break
        if (env.event.k === 'LspApplyEdit') {
          if (env.workspace.hostId === (get().remoteState.hostId ?? 'local')) window.dispatchEvent(new CustomEvent('koma-lsp-edit', { detail: { workspace: env.workspace, ticket: env.event.ticket } }))
          else void codingRequest(env.workspace, { op: 'lspEditReply', ticket: String(env.event.ticket), applied: false, reason: 'The originating host is not active in this window' }).catch(() => {})
          break
        }
        if (env.event.k === 'LspCommandResult') {
          if (env.event.error) set(s => { const id = s.ui.toastSeq + 1; return { ui: { ...s.ui, toastSeq: id, toast: { id, kind: 'error', text: `${env.workspace.hostId} · ${env.workspace.root}: ${String(env.event.error)}` } } } })
          break
        }
        if (env.workspace.hostId !== (get().remoteState.hostId ?? 'local')) break
        if (env.event.k === 'LspTransportConnected') {
          window.dispatchEvent(new CustomEvent('koma-lsp-restart', { detail: { hostId: env.workspace.hostId } }))
          break
        }
        if (env.event.k === 'LspWorkspaceRestart') {
          window.dispatchEvent(new CustomEvent('koma-lsp-restart', { detail: env.workspace }))
          break
        }
        if (env.event.k === 'FileSystemChanged') {
          window.dispatchEvent(new CustomEvent('koma-coding-disk', { detail: env.workspace }))
          break
        }
        const event = env.event as PushEnvelope
        // Only language events are accepted from a coding worker.
        if (!event.k.startsWith('Lsp')) break
        if (event.k === 'LspRuntime' && event.replace) {
          const incoming = new Set(event.servers.map(row => row.id))
          get().push({ ...event, replace: false, removed: [...(event.removed ?? []), ...get().lspRuntime.filter(row => row.root === env.workspace.root && !incoming.has(row.id)).map(row => row.id)] })
        } else get().push(event)
        break
      }
      case 'FileDiff':
        set((s) => {
          const id = `diff:${env.path}`
          // Ignore a reply for a tab closed while the req was in flight.
          if (!s.ui.tabs.some((t) => t.id === id)) return s
          return {
            ui: {
              ...s.ui,
              tabs: s.ui.tabs.map((t) =>
                t.id === id && t.kind === 'diff'
                  ? {
                      ...t,
                      loading: false,
                      diff: {
                        original: env.original,
                        modified: env.modified,
                        error: env.error,
                        binary: env.binary,
                        origin: env.origin ?? 'git',
                      },
                    }
                  : t,
              ),
            },
          }
        })
        break
      case 'LspStatus':
        set(() => ({
          lspServers: env.servers,
          // Clear progress for servers that are no longer installing.
          lspProgress: {},
        }))
        break
      case 'LspInstall':
        set((s) => {
          const text = env.error ? `lsp ${env.id || 'install'}: ${env.error}` : null
          const raise = !!text && text !== s.ui.toast?.text
          const seq = raise ? s.ui.toastSeq + 1 : s.ui.toastSeq
          const nextProgress = { ...s.lspProgress }
          if (env.id) {
            if (env.error || env.pct >= 100) {
              // Keep a brief terminal state; status push will clear.
              nextProgress[env.id] = { id: env.id, pct: env.pct, error: env.error }
            } else {
              nextProgress[env.id] = { id: env.id, pct: env.pct, error: env.error }
            }
          }
          return {
            lspProgress: nextProgress,
            ui: raise
              ? { ...s.ui, toastSeq: seq, toast: { id: seq, text: text as string, kind: 'error' } }
              : s.ui,
          }
        })
        break
      case 'LspDiagnostics':
        set((s) => {
          const prev = s.lspDiagnostics[env.uri] ?? []
          const next = env.diagnostics ?? []
          let errors = s.lspDiagCounts.errors
          let warnings = s.lspDiagCounts.warnings
          for (const d of prev) {
            if (d.severity === 1) errors -= 1
            else if (d.severity === 2) warnings -= 1
          }
          for (const d of next) {
            if (d.severity === 1) errors += 1
            else if (d.severity === 2) warnings += 1
          }
          if (errors < 0) errors = 0
          if (warnings < 0) warnings = 0
          return {
            lspDiagnostics: {
              ...s.lspDiagnostics,
              [env.uri]: next,
            },
            lspDiagCounts: { errors, warnings },
          }
        })
        // Markers only — coding tabs must not subscribe to the full map.
        // Lazy: monaco-lsp is not in the boot bundle.
        void import('../../lib/monaco-lsp')
          .then((m) => m.applyDiagnosticsToMonaco(env.uri, env.diagnostics ?? []))
          .catch(() => {
            /* monaco not loaded yet */
          })
        break
      case 'LspCompletion':
        resolveLspCompletion(env.requestId, env.items ?? [], env.error, env.isIncomplete)
        break
      case 'LspCompletionResolve':
        resolveLspCompletionResolve(env.requestId, env.item ?? null, env.error)
        break
      case 'LspHover':
        resolveLspHover(env.requestId, env.hover ?? null, env.error)
        break
      case 'LspDefinition':
        resolveLspDefinition(env.requestId, env.locations ?? [], env.error)
        break
      case 'LspReferences':
        resolveLspReferences(env.requestId, env.locations ?? [], env.error)
        break
      case 'LspDocumentSymbol':
        resolveLspDocumentSymbol(env.requestId, env.symbols ?? [], env.error)
        break
      case 'LspRuntime':
        set((s) => {
          const incoming = (env.servers ?? []).map((row) => ({
            id: row.id,
            name: row.name,
            root: row.root ?? '',
            phase: row.phase || 'ready',
            title: row.title ?? null,
            message: row.message ?? null,
            percentage: row.percentage ?? null,
            openDocs: row.openDocs ?? 0,
          }))
          const removed = new Set(env.removed ?? [])
          if (env.replace) {
            return { lspRuntime: incoming.filter((r) => !removed.has(r.id)) }
          }
          const byId = new Map(s.lspRuntime.map((r) => [r.id, r]))
          for (const id of removed) byId.delete(id)
          for (const row of incoming) {
            const prev = byId.get(row.id)
            // Reader-thread deltas don't know open-doc counts — keep the last
            // control-loop value when the delta reports 0 and we already have one.
            const openDocs =
              row.openDocs > 0 || !prev
                ? row.openDocs
                : prev.openDocs
            byId.set(row.id, { ...row, openDocs })
          }
          const next = Array.from(byId.values()).sort((a, b) =>
            a.name.localeCompare(b.name) || a.id.localeCompare(b.id),
          )
          return { lspRuntime: next }
        })
        break
      case 'FileTree':
        set((s) => ({ coding: reduceFileTree(s.coding, env) }))
        break
      case 'FileRead': {
        const claimed = claimDiagramRead(get().diagram, env)
        if (claimed) {
          set({ diagram: claimed.diagram })
          if (claimed.save) {
            get().req({
              r: 'FileSave',
              root: claimed.save.root,
              path: claimed.save.path,
              content: claimed.save.content,
              expectedFingerprint: claimed.save.fingerprint,
              requestId: claimed.save.requestId,
            })
          }
          break
        }
        resolveLspFileText(
          env.requestId,
          env.content ?? null,
          env.error,
          env.binary,
          env.tooLarge,
        )
        set((s) => ({ coding: reduceFileRead(s.coding, env) }))
        break
      }
      case 'FileSave': {
        const diagramSaved = claimDiagramSave(get().diagram, env)
        if (diagramSaved) {
          set({ diagram: diagramSaved })
          break
        }
        const key = fileKey(env.root, env.path)
        const before = get().coding.files[key]
        const pending = before?.pendingSave
        if (!pending || pending.requestId !== env.requestId) break
        set((s) => ({ coding: reduceFileSave(s.coding, env) }))
        if (!env.error) {
          const workspace = { hostId: get().remoteState.hostId ?? 'local', root: env.root }
          if (env.path === '.koma/coding.json') invalidateCodingConfig(workspace)
          if (before.savedContent != null) recordCodingHistory(workspace, env.path, before.savedContent, 'Before save')
          recordCodingHistory(workspace, env.path, pending.content, 'Saved')
          const saved = get().coding.files[key]
          if (saved?.dirty && saved.content != null) backupCodingDocument(workspace, env.path, {
            content: saved.content, savedContent: saved.savedContent, fingerprint: saved.fingerprint,
          })
          else forgetCodingDraft(workspace, env.path)
          get().req({
            r: 'LspDidSave',
            root: env.root,
            path: env.path,
            text: pending.content,
          })
          // didSave describes disk, but the open LSP document may already be
          // newer. Reassert that buffer after the save notification.
          const latest = get().coding.files[key]?.content
          if (latest != null && latest !== pending.content) {
            get().req({ r: 'LspDidChange', root: env.root, path: env.path, text: latest })
          }
          if (before.saveQueued && get().coding.files[key]?.dirty) {
            get().saveCodingFile(env.root, env.path)
          }
        }
        break
      }
      case 'FileContentSearch':
        set((s) => ({ coding: reduceFileContentSearch(s.coding, env) }))
        break
      case 'FileContentReplace':
        set((s) => ({ coding: reduceFileContentReplace(s.coding, env) }))
        if (!env.error && env.filesChanged > 0) {
          // Disk changed under open buffers — force-reload open non-dirty files in this root.
          const root = env.root
          const open = get().ui.tabs.filter(
            (t): t is Extract<Tab, { kind: 'codingFile' }> =>
              t.kind === 'codingFile' && t.root === root,
          )
          for (const t of open) {
            const f = get().coding.files[fileKey(root, t.path)]
            if (f && !f.dirty) {
              get().openCodingFile(root, t.path, { force: true, preview: t.preview })
            }
          }
          // Re-run search so results reflect post-replace state.
          queueMicrotask(() => get().searchCodingContent(root))
        }
        break
      case 'FileCreate':
        set((s) => {
          const coding = reduceFileCreate(s.coding, env)
          if (env.error) {
            const seq = s.ui.toastSeq + 1
            return {
              coding,
              ui: {
                ...s.ui,
                toastSeq: seq,
                toast: { id: seq, text: env.error, kind: 'error' },
              },
            }
          }
          queueMicrotask(() => {
            const parent = env.path.includes('/')
              ? env.path.slice(0, env.path.lastIndexOf('/'))
              : ''
            get().refreshCodingDir(env.root, parent)
          })
          return { coding }
        })
        {
          const pending = get().diagram.pendingCreate
          if (pending && pending.createReq === env.requestId) {
            if (env.error) {
              set((s) => ({
                diagram: {
                  ...s.diagram,
                  pendingCreate: s.diagram.pendingCreate?.createReq === env.requestId ? null : s.diagram.pendingCreate,
                },
              }))
            } else {
              beginDiagramSeed(set, get, pending.root, pending.path)
            }
          }
        }
        break
      case 'FileRename':
        set((s) => {
          const coding = reduceFileRename(s.coding, env)
          if (env.error) {
            const seq = s.ui.toastSeq + 1
            return {
              coding,
              ui: {
                ...s.ui,
                toastSeq: seq,
                toast: { id: seq, text: env.error, kind: 'error' },
              },
            }
          }
          // Remap codingFile tabs for the renamed path and every descendant.
          let activeTabId = s.ui.activeTabId
          const tabGroup = { ...s.ui.tabGroup }
          const groupActive = { ...s.ui.groupActive }
          const tabs = s.ui.tabs.map((t) => {
            if ((t.kind !== 'codingFile' && t.kind !== 'diagram') || t.root !== env.root) return t
            const mapped = codingRemapPath(t.path, env.oldPath, env.newPath)
            if (mapped == null) return t
            const newId = t.kind === 'diagram' ? diagramTabId(env.root, mapped) : codingTabId(env.root, mapped, t.preview)
            if (s.ui.activeTabId === t.id) activeTabId = newId
            if (tabGroup[t.id]) {
              tabGroup[newId] = tabGroup[t.id]
              delete tabGroup[t.id]
            }
            for (const gid of Object.keys(groupActive)) {
              if (groupActive[gid] === t.id) groupActive[gid] = newId
            }
            return {
              ...t,
              id: newId,
              path: mapped,
              title: codingBaseName(mapped),
            }
          })
          queueMicrotask(() => {
            const parents = new Set([
              env.oldPath.includes('/')
                ? env.oldPath.slice(0, env.oldPath.lastIndexOf('/'))
                : '',
              env.newPath.includes('/')
                ? env.newPath.slice(0, env.newPath.lastIndexOf('/'))
                : '',
            ])
            for (const p of parents) get().refreshCodingDir(env.root, p)
          })
          return {
            coding,
            diagram: { ...s.diagram, docs: remapDiagramDocs(s.diagram.docs, env.root, env.oldPath, env.newPath) },
            ui: normalizeGroups({ ...s.ui, tabs, activeTabId, tabGroup, groupActive }),
          }
        })
        break
      case 'FileDelete':
        set((s) => {
          const coding = reduceFileDelete(s.coding, env)
          if (env.error) {
            const seq = s.ui.toastSeq + 1
            return {
              coding,
              ui: {
                ...s.ui,
                toastSeq: seq,
                toast: { id: seq, text: env.error, kind: 'error' },
              },
            }
          }
          // Close codingFile tabs for the deleted path and every descendant.
          const closed = new Set<string>()
          const tabs = s.ui.tabs.filter((t) => {
            if ((t.kind !== 'codingFile' && t.kind !== 'diagram') || t.root !== env.root) return true
            if (!codingIsPathOrDescendant(t.path, env.path)) return true
            closed.add(t.id)
            return false
          })
          let activeTabId = s.ui.activeTabId
          if (closed.has(s.ui.activeTabId)) {
            // Prefer the previous surviving tab; fall back to chat.
            const idx = s.ui.tabs.findIndex((t) => t.id === s.ui.activeTabId)
            let pick: string | null = null
            for (let i = idx - 1; i >= 0; i--) {
              if (!closed.has(s.ui.tabs[i].id)) {
                pick = s.ui.tabs[i].id
                break
              }
            }
            if (!pick) {
              for (let i = idx + 1; i < s.ui.tabs.length; i++) {
                if (!closed.has(s.ui.tabs[i].id)) {
                  pick = s.ui.tabs[i].id
                  break
                }
              }
            }
            activeTabId = pick ?? 'chat'
          }
          queueMicrotask(() => {
            const parent = env.path.includes('/')
              ? env.path.slice(0, env.path.lastIndexOf('/'))
              : ''
            get().refreshCodingDir(env.root, parent)
          })
          return {
            coding,
            diagram: { ...s.diagram, docs: dropDiagramDocs(s.diagram.docs, env.root, env.path) },
            ui: { ...s.ui, tabs, activeTabId },
          }
        })
        break
      case 'FileWriteBytes':
        set((s) => {
          const coding = reduceFileWriteBytes(s.coding, env)
          if (env.error) {
            const seq = s.ui.toastSeq + 1
            return {
              coding,
              ui: {
                ...s.ui,
                toastSeq: seq,
                toast: { id: seq, text: env.error, kind: 'error' },
              },
            }
          }
          queueMicrotask(() => {
            const parent = env.path.includes('/')
              ? env.path.slice(0, env.path.lastIndexOf('/'))
              : ''
            get().refreshCodingDir(env.root, parent)
          })
          return { coding }
        })
        break
      case 'FileDownloadBytes': {
        // Preview waiters (CodingFileViewer) consume first — skip save-as UI.
        if (
          resolveFilePreviewBytes(
            env.requestId,
            env.bytesB64,
            env.error,
            env.tooLarge,
          )
        ) {
          break
        }
        if (env.error || env.tooLarge) {
          const text =
            env.error ||
            (env.tooLarge ? 'file too large to download' : 'download failed')
          set((s) => {
            const seq = s.ui.toastSeq + 1
            return {
              ui: {
                ...s.ui,
                toastSeq: seq,
                toast: { id: seq, text, kind: 'error' },
              },
            }
          })
          break
        }
        // Host-native save-as (saveAs:true on the request) already wrote the
        // file via rfd. Blob <a download> is a no-op in wry, so this is the
        // only path that actually lands a file on disk.
        if (env.saved) {
          const name = codingBaseName(env.path) || 'download'
          set((s) => {
            const seq = s.ui.toastSeq + 1
            return {
              ui: {
                ...s.ui,
                toastSeq: seq,
                toast: { id: seq, text: `saved ${name}`, kind: 'info' },
              },
            }
          })
          break
        }
        // Cancelled dialog, or a non-saveAs reply that nobody claimed —
        // nothing to do (no toast on cancel).
        break
      }
      case 'TerminalOutput':
        // Route PTY output to the xterm.js instance via the global write callback.
        // The TerminalTab component registers its write function on mount.
        {
          const writer = (globalThis as any).__terminalWriters?.[env.id]
          if (writer) writer(env.data)
        }
        break
      case 'TerminalExit':
        // Mark the terminal as exited via the global exit callback.
        {
          const handler = (globalThis as any).__terminalExitHandlers?.[env.id]
          if (handler) handler(env.code)
        }
        break
    default:
      return false
  }
  return true
}
