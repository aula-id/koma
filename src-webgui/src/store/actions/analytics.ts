import type { StoreGet, StoreSet } from '../api'
import type { KomaState } from '../state'
import type { AnalyticsScope } from '../types/analytics'
import type { Tab } from '../types/tabs'

export function analyticsActions(set: StoreSet, get: StoreGet): Pick<KomaState, 'openAnalyticsTab' | 'refreshAnalytics' | 'setAnalyticsScope' | 'setAnalyticsRange' | 'setAnalyticsMetric'> {
  return {
  openAnalyticsTab: () => {
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === 'analytics')
      const tabs: Tab[] = exists
        ? s.ui.tabs
        : [...s.ui.tabs, { id: 'analytics', kind: 'analytics' }]
      return { ui: { ...s.ui, tabs, activeTabId: 'analytics' } }
    })
    // No wire fetch here — the AnalyticsTab fires refreshAnalytics on mount.
  },
  refreshAnalytics: () => {
    const a = get().analytics
    const sessionId = get().session.id
    // Welcome-screen rule: force "all" when there's no session to filter by.
    const scope: AnalyticsScope =
      a.scope === 'session' && sessionId === null ? 'all' : a.scope
    const reqSeq = a.reqSeq + 1
    const requestedSessionId = scope === 'session' ? sessionId : null
    set((s) => ({
      analytics: {
        ...s.analytics,
        scope,
        reqSeq,
        sessionId: requestedSessionId,
        loading: true,
        error: null,
        // Keep previous data visible while loading so filter flips don't flash
        // an empty shell; the reducer replaces it only when the matching reply
        // lands. Clear hasData only when filters actually changed would be
        // nicer, but keeping the previous payload is the UsagePreview pattern
        // inverted — Analytics prefers keep-stale-until-fresh.
      },
    }))
    get().req({
      r: 'Analytics',
      reqSeq,
      scope,
      sessionId: requestedSessionId ?? undefined,
      range: a.range,
      metric: a.metric,
    })
  },
  setAnalyticsScope: (scope) => {
    // Force "all" when there's no session (welcome screen).
    const sessionId = get().session.id
    const next: AnalyticsScope = scope === 'session' && sessionId === null ? 'all' : scope
    set((s) => ({
      analytics: {
        ...s.analytics,
        scope: next,
        // Drop previous data immediately on a filter change so the wrong
        // scope/range never flashes under the new selection.
        data: null,
        hasData: false,
        error: null,
      },
    }))
    get().refreshAnalytics()
  },
  setAnalyticsRange: (range) => {
    set((s) => ({
      analytics: {
        ...s.analytics,
        range,
        data: null,
        hasData: false,
        error: null,
      },
    }))
    get().refreshAnalytics()
  },
  setAnalyticsMetric: (metric) => {
    set((s) => ({
      analytics: {
        ...s.analytics,
        metric,
        // Metric only rescales the same series — keep data, no re-fetch needed
        // host-side (series carries both cost and tokens). Still bump nothing;
        // just re-render.
      },
    }))
  },
  }
}
