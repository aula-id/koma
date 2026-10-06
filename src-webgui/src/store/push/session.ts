import { cancelGitRequests } from '../../lib/gitWorkbench'
import type { StoreGet, StoreSet } from '../api'
import { useComputerPreview } from '../computerPreview'
import { normalizeGroups } from '../editorGroups'
import { applyPaletteVars, initialActivity, initialGit, initialGraph, initialImportGraph, makeBootstrapState, makeChatTab } from '../initial'
import type { KomaState } from '../state'
import { useKoma } from '../koma'
import type { ChatMessage } from '../types/chat'
import type { PushEnvelope } from '../types/envelope'

export function pushSession(set: StoreSet, get: StoreGet, env: PushEnvelope): boolean {
  switch (env.k) {
      case 'Snapshot': {
        // A Snapshot whose session id differs from the current one is a session
        // SWITCH. Captured BEFORE the set so it's readable AFTER (to sync the stream
        // view). The set reuses it to drop the OLD session's in-flight stream/
        // reasoning (it belongs to the old session — don't let it bleed into the new
        // view until the next send clears it) + reset the editor tabs.
        const previousSessionId = get().session.id
        const switched = env.session !== previousSessionId
        const preservedTabLayout = get().ui.preservedTabLayout
        const preserveEditorTabs = get().ui.preserveTabsOnNextSession || preservedTabLayout !== null
        const targetSession = get().ui.preservedTabsTargetSession
        const consumePreservedTabs = preserveEditorTabs && targetSession === env.session
        if (switched) {
          cancelGitRequests()
          useComputerPreview.getState().hide()
          set({ computer: null, computerError: null })
          // Preview.7 releases host resources on a real A→B switch. A guided
          // detached first-chat attach must retain its captured editor layout.
          if (previousSessionId !== null && !preserveEditorTabs) {
            get().closeAllTabsExceptChat({ force: true })
          }
        }
        // Re-attaching the same session id is still a GUI bootstrap even though
        // it must not discard that session's existing tabs/slices.
        const bootstrapping = !!get().ui.bootstrap
        set((s) => {
          const uiBeforeSnapshot = preservedTabLayout
            ? {
                ...s.ui,
                ...preservedTabLayout,
                tabs: preservedTabLayout.tabs.map((tab) => ({ ...tab })),
                tabGroup: { ...preservedTabLayout.tabGroup },
                groupActive: { ...preservedTabLayout.groupActive },
                groupSizes: { ...preservedTabLayout.groupSizes },
                groupSplitDir: { ...preservedTabLayout.groupSplitDir },
              }
            : s.ui
          return {
            session: {
              ...s.session,
              id: env.session,
              state: env.state,
              messages: (env.messages ?? []).map((m: any) => ({
                ...m,
                toolCalls: m.toolCalls ?? m.tool_calls,
              })),
              messageCount:
                typeof env.messageCount === 'number'
                  ? env.messageCount
                  : (env.messages ?? []).length,
              hasMoreOlder:
                typeof env.messageCount === 'number'
                  ? env.messageCount > (env.messages ?? []).length
                  : false,
              title: env.title,
              subagents: env.subagents,
              bash: env.bash,
              // Defensive fallback: tolerates a host build that hasn't started
              // projecting fileChanges[] on the Snapshot envelope yet.
              fileChanges: env.fileChanges ?? [],
              // planTodos is adopted below (mode-gated to prevent stale Plan
              // rows from bleeding into non-plan modes).
              // Defensive fallback: tolerates a host build that hasn't started
              // projecting attachments[] on the Snapshot envelope yet.
              attachments: env.attachments ?? [],
              // Adopt the projected agent mode when present; keep the current
              // one otherwise (host build not projecting it yet).
              mode: env.mode ?? s.session.mode,
              // Adopt the projected pending-steer queue; defensive fallback for a
              // host build that doesn't project it yet (empty queue).
              pendingSteer: env.pendingSteer ?? [],
              // Adopt the projected approval gate; defensive fallbacks for a host
              // build that doesn't project it yet (gate closed, no pending call).
              awaitingApproval: env.awaitingApproval ?? false,
              modelRoutes: env.modelRoutes ?? [],
              approvalReason: env.approvalReason ?? null,
              pendingCall: env.pendingCall ?? null,
              // Adopt SDLC fields from the snapshot, mode-gated defensively:
              // the host SHOULD only send these when mode=sdlc (and clears them
              // otherwise), but a malformed/old host might not — so the store
              // enforces the invariant client-side too.
              sdlcPhase: env.mode === 'sdlc' ? (env.sdlcPhase ?? null) : null,
              sdlcGoal: env.mode === 'sdlc' ? (env.sdlcGoal ?? null) : null,
              sdlcBranch: env.mode === 'sdlc' ? (env.sdlcBranch ?? null) : null,
              sdlcOpen: env.mode === 'sdlc' ? (env.sdlcOpen ?? null) : null,
              sdlcSealed: env.mode === 'sdlc' ? (env.sdlcSealed ?? null) : null,
              // Plan/SDLC checklist rows: plan mode = plan_todos.md; sdlc = L2 graph projection.
              // Clear stale rows outside those modes (never leak Auto/Normal).
              planTodos: (env.mode === 'plan' || env.mode === 'sdlc')
                ? (env.planTodos ?? []).map((t) => ({ ...t, locked: t.locked ?? false }))
                : [],
              ...(switched ? { stream: '', reasoning: '' } : {}),
            },
            palette: env.palette,
            loadedSkillNames: env.loadedSkillNames ?? [],
            // Snapshot proves attach landed — drop switch chrome immediately so
            // chat can paint. Do NOT invent a synthetic Loading splash here:
            // that held "indexing workspace" over WebKit while the fat Snapshot
            // still parsed, freezing Mac/Linux reopen. Real warm-up still shows
            // when the host emits Loading{active:true}.
            ui: normalizeGroups<KomaState['ui']>({
              ...uiBeforeSnapshot,
              switchingTo: null,
              preserveTabsOnNextSession: preserveEditorTabs && !consumePreservedTabs,
              preservedTabLayout: consumePreservedTabs ? null : preservedTabLayout,
              preservedTabsTargetSession: consumePreservedTabs ? null : targetSession,
              ...(switched || bootstrapping
                ? {
                    ...(switched && previousSessionId !== null && !preserveEditorTabs
                      ? {
                          tabs: [makeChatTab()],
                          activeTabId: 'chat',
                        }
                      : {}),
                    // Keep a real host Loading envelope; never synthesize pending.
                    loading: s.ui.loading?.active ? s.ui.loading : null,
                    loadingDismissed: s.ui.loading?.active ? false : s.ui.loadingDismissed,
                    bootstrap: {
                      ...(s.ui.bootstrap ?? makeBootstrapState()),
                      session: 'done',
                      config: s.ui.bootstrap?.config === 'done' ? 'done' : 'running',
                      settings: s.ui.bootstrap?.settings === 'done' ? 'done' : 'running',
                      repos: s.ui.bootstrap?.repos === 'done' ? 'done' : 'running',
                    },
                  }
                : {}),
            }),
            // A genuine switch also drops the OLD session's git/graph/activity
            // slices — they're host-driven for the PREVIOUS repo/session and
            // must not bleed into the new one until each panel's own
            // session-keyed effect re-fetches. The graph view MODE
            // (rail/bubble) is a user preference, not session data, so it's
            // preserved across the reset.
            ...(switched
              ? {
                  git: initialGit,
                  activity: initialActivity,
                  // Preserve Analytics filters (user preference) but drop any
                  // session-scoped result so session A's numbers never render
                  // under session B. All-scope data stays valid across switch.
                  analytics: {
                    ...s.analytics,
                    ...(s.analytics.scope === 'session'
                      ? {
                          data: null,
                          hasData: false,
                          loading: false,
                          error: null,
                          sessionId: null,
                        }
                      : {}),
                  },
                  graph: { ...initialGraph, graphMode: s.graph.graphMode },
                  importGraph: initialImportGraph,
                  // Coding documents belong to the host/workspace, not chat.
                  repos: [],
                  activeRepoRoot: null,
                  skills: [],
                  skillsLoading: false,
                  skillsError: null,
                  skillsUnconfirmed: null,
                  skillRequestId: null,
                  skillSessionEpoch: s.skillSessionEpoch + 1,
                  skillSelection: [],
                  skillDetails: {},
                  skillDetailPending: {},
                  skillDetailErrors: {},
                  skillFiles: {},
                  skillOutcomes: [],
                  skillLastOp: null,
                  skillOpResults: {},
                  skillDeletePending: {},
                }
              : {}),
          }
        })
        applyPaletteVars(env.palette)
        // A genuine switch reset the tabs to just chat (above) → no stream tab is
        // active. Sync the now-empty stream view so the host stops streaming the OLD
        // session's sub-agent/bash target (the new session's daemon starts with none
        // anyway). Fired AFTER the set so it reads the reset tab state.
        if (switched) get().syncStreamView()
        // Preserved Skill tabs outlive the session-scoped catalogue/detail cache.
        // Re-discover in the NEW chat's workdir, then each mounted SkillTab can
        // request its detail at the new epoch. Do not discover for an unrelated
        // in-flight Snapshot while a guided chat's target is still pending.
        if (switched && (!preserveEditorTabs || consumePreservedTabs) &&
            get().ui.tabs.some((tab) => tab.kind === 'skill' && tab.skillId !== null)) {
          get().refreshSkills()
        }
        // Settings / repos after the Snapshot turn so they cannot join the fat
        // apply and freeze the overlay paint.
        if (switched || bootstrapping) {
          requestAnimationFrame(() => {
            get().req({ r: 'GetSettings' })
            get().refreshRepos()
          })
        }
        break
      }
      case 'Switching':
        useComputerPreview.getState().hide()
        set({ computer: null, computerError: null })
        cancelGitRequests()
        set((s) => {
          // Prefer an optimistic label ResumePalette already raised (the
          // friendly name the user clicked); otherwise resolve the target id
          // against the hub rows; else fall back to a generic label (e.g. a
          // daemon-driven new session with no hub row yet). Never clobber a
          // nicer label with a raw uuid.
          if (s.ui.switchingTo && s.ui.bootstrap && !s.ui.preserveTabsOnNextSession) return s
          const row =
            s.hub.cooking.find((c) => c.id === env.to) ??
            s.hub.history.find((h) => h.id === env.to)
          return {
            ui: {
              ...s.ui,
              switchingTo: s.ui.switchingTo ?? row?.name ?? 'session',
              preservedTabsTargetSession: s.ui.preserveTabsOnNextSession ? env.to : s.ui.preservedTabsTargetSession,
              bootstrap: s.ui.bootstrap ?? makeBootstrapState(),
              loadingDismissed: false,
            },
          }
        })
        break
      case 'SnapshotTail': {
        // Same-session append-only growth. Ignore if session drifted (full
        // Snapshot will resync). Map planTodos-style fields aren't present.
        if (env.session !== get().session.id) break
        const mapped = (env.messages ?? []).map((m: any) => ({
          ...m,
          toolCalls: m.toolCalls ?? m.tool_calls,
        }))
        if (mapped.length === 0) break
        set((s) => ({
          session: {
            ...s.session,
            messages: s.session.messages.concat(mapped),
            messageCount: Math.max(
              s.session.messageCount,
              s.session.messages.length + mapped.length,
            ),
          },
        }))
        break
      }
      case 'SnapshotHead': {
        if (env.session !== get().session.id) break
        const mapped = (env.messages ?? []).map((m: any) => ({
          ...m,
          toolCalls: m.toolCalls ?? m.tool_calls,
        }))
        if (mapped.length === 0) break
        // Prepend one host chunk. Multi-chunk heads arrive on later frames;
        // ChatView shifts renderFrom on large length jumps so the tail stays put.
        set((s) => {
          const seen = new Set<number>()
          for (const m of s.session.messages) {
            if (typeof m.idx === 'number') seen.add(m.idx)
          }
          const fresh = mapped.filter(
            (m: ChatMessage) => typeof m.idx !== 'number' || !seen.has(m.idx as number),
          )
          const messages =
            fresh.length === 0 ? s.session.messages : fresh.concat(s.session.messages)
          const hasMoreOlder =
            env.more === true ||
            messages.length < (s.session.messageCount || messages.length)
          return {
            session: {
              ...s.session,
              messages,
              hasMoreOlder,
            },
          }
        })
        break
      }
      case 'HistoryPage': {
        if (env.session !== get().session.id) break
        const mapped = (env.messages ?? []).map((m: any) => ({
          ...m,
          toolCalls: m.toolCalls ?? m.tool_calls,
        }))
        set((s) => {
          const seen = new Set<number>()
          for (const m of s.session.messages) {
            if (typeof m.idx === 'number') seen.add(m.idx)
          }
          const fresh = mapped.filter(
            (m: ChatMessage) => typeof m.idx !== 'number' || !seen.has(m.idx as number),
          )
          const messages =
            fresh.length === 0 ? s.session.messages : fresh.concat(s.session.messages)
          return {
            session: {
              ...s.session,
              messages,
              hasMoreOlder: !!env.hasMore,
              messageCount: Math.max(s.session.messageCount, messages.length),
            },
          }
        })
        break
      }
      case 'SnapshotSetLast': {
        if (env.session !== get().session.id) break
        const m: any = env.message
        if (!m) break
        const mapped = {
          ...m,
          toolCalls: m.toolCalls ?? m.tool_calls,
        }
        set((s) => {
          const msgs = s.session.messages
          if (msgs.length === 0) return s
          const next = msgs.slice()
          next[next.length - 1] = mapped
          return { session: { ...s.session, messages: next } }
        })
        break
      }
      case 'StreamMsg':
        set((s) => ({ session: { ...s.session, stream: env.text } }))
        break
      case 'StreamDelta':
        set((s) => ({
          session: {
            ...s.session,
            stream: env.reset ? env.append : s.session.stream + env.append,
          },
        }))
        break
      case 'Reasoning':
        set((s) => ({ session: { ...s.session, reasoning: env.text } }))
        break
      case 'ReasoningDelta':
        set((s) => ({
          session: {
            ...s.session,
            reasoning: env.reset ? env.append : s.session.reasoning + env.append,
          },
        }))
        break
      case 'Status':
        set((s) => {
          // Only raise a NEW toast when the text actually changed from the one
          // already showing — the host re-pushes the same live toast on every
          // Status tick (it has a host-side TTL), so deduping by text keeps the
          // dismiss timer from being reset on each tick. A cleared toast
          // (env.toast null) never wipes an active card; the auto-dismiss owns
          // that so a working=false status can't cut a toast short.
          const raise = !!env.toast && env.toast !== s.ui.toast?.text
          const seq = raise ? s.ui.toastSeq + 1 : s.ui.toastSeq
          const newMode = env.mode ?? s.session.mode
          const modeChanged = newMode !== s.session.mode
          return {
            session: {
              ...s.session,
              working: env.working,
              // Usage counters + mode ride the Status envelope too (not just
              // Snapshot), so the footer updates live mid-turn. Optional-
              // tolerant: an older host build omits these — keep the current
              // value rather than resetting to 0/'auto' on every tick.
              tokensIn: env.tokensIn ?? s.session.tokensIn,
              tokensCached: env.tokensCached ?? s.session.tokensCached,
              tokensOut: env.tokensOut ?? s.session.tokensOut,
              cost: env.cost ?? s.session.cost,
              contextWindow: env.contextWindow ?? s.session.contextWindow,
              mode: newMode,
              // Explicitly clear stale SDLC rows on mode change: SDLC fields
              // are only valid when mode=sdlc; a mode switch must not leave
              // stale phase/goal/branch/counts from a previous SDLC session.
              // Also clear Plan rows when leaving Plan mode (same invariant).
              // When mode didn't change, preserve current values (the next
              // Snapshot will overwrite them authoritatively anyway).
              ...(modeChanged
                ? {
                    sdlcPhase: newMode === 'sdlc' ? s.session.sdlcPhase : null,
                    sdlcGoal: newMode === 'sdlc' ? s.session.sdlcGoal : null,
                    sdlcBranch: newMode === 'sdlc' ? s.session.sdlcBranch : null,
                    sdlcOpen: newMode === 'sdlc' ? s.session.sdlcOpen : null,
                    sdlcSealed: newMode === 'sdlc' ? s.session.sdlcSealed : null,
                    planTodos: (newMode === 'plan' || newMode === 'sdlc') ? s.session.planTodos : [],
                  }
                : {}),
            },
            ui: raise
              ? {
                  ...s.ui,
                  toastSeq: seq,
                  toast: {
                    id: seq,
                    text: env.toast as string,
                    // Pass a recognised severity straight through (future-proofs
                    // "warn"/"success" if the host ever emits them); anything else
                    // (today: everything but "error") falls back to "info".
                    kind:
                      env.toastKind === 'error' || env.toastKind === 'warn' || env.toastKind === 'success'
                        ? env.toastKind
                        : 'info',
                  },
                }
              : s.ui,
          }
        })
        break
      case 'UsageLive':
        set((s) => ({
          session: {
            ...s.session,
            memWindow: env.memWindow ?? s.session.memWindow,
            memAgent: env.memAgent ?? s.session.memAgent,
            memServices: env.memServices ?? s.session.memServices,
            memSystem: env.memSystem ?? s.session.memSystem,
          },
        }))
        break
      case 'Hub':
        set((s) => {
          // Prune "dying" marks the moment a fresh Hub push confirms the
          // matching disposition landed. Kind-scoped: a killed session stays
          // on disk and MIGRATES from cooking to history on this very push —
          // so a 'kill' mark clears when the id drops out of COOKING
          // (regardless of it now appearing in history), and a 'delete' mark
          // clears when the id drops out of HISTORY. An id-agnostic
          // "absent from both lists" rule would keep a migrated-in history
          // row stuck spinning forever (the real bug this fixes).
          const cookingIds = new Set<string>(
            env.cooking.map((c) => c.id).filter((id): id is string => !!id),
          )
          const historyIds = new Set<string>(env.history.map((h) => h.id))
          // Hub while switching: only clear switch chrome when the swap truly
          // bounced back to the swapper. StartScreen (and ResumePalette) poll
          // RefreshHub on an interval; those replies can land mid-attach and
          // must NOT tear down switchingTo / Loading / bootstrap. Treat Hub as
          // a bounce only when we are still detached (no session id) AND not
          // already in post-attach warm-up (loading/bootstrap). Real
          // attach-failure paths re-enter host_swapper with session.id still
          // null and no Loading frame, so they still clear correctly.
          const midAttach =
            !!s.ui.switchingTo &&
            (!!s.session.id || !!s.ui.loading?.active || !!s.ui.bootstrap)
          return {
            hub: { ...s.hub, state: env.state, cooking: env.cooking, history: env.history },
            dyingSessions: s.dyingSessions.filter((d) =>
              d.kind === 'kill' ? cookingIds.has(d.id) : historyIds.has(d.id),
            ),
            ...(midAttach || !s.ui.switchingTo
              ? {}
              : {
                  ui: { ...s.ui, switchingTo: null, preserveTabsOnNextSession: false, preservedTabLayout: null, preservedTabsTargetSession: null, loading: null, bootstrap: null },
                }),
          }
        })
        break
      case 'SearchResults':
        set((s) => ({ session: { ...s.session, searchResults: env.items } }))
        break
      case 'PasteBody':
        set((s) => ({ ui: { ...s.ui, pasteBody: { markerN: env.markerN, text: env.text } } }))
        break
      case 'AttachmentLocated':
        useKoma.getState().openLocalFileTab(env.absPath, env.name)
        break
    default:
      return false
  }
  return true
}
