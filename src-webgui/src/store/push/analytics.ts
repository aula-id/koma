import type { StoreGet, StoreSet } from '../api'
import type { PushEnvelope } from '../types/envelope'

export function pushAnalytics(set: StoreSet, _get: StoreGet, env: PushEnvelope): boolean {
  switch (env.k) {
      case 'UsagePreview':
        set((s) => {
          // Drop a reply for a scope the user has since switched away from (a
          // rapid all/session toggle racing an in-flight request) — leave
          // `usagePreview` as-is (likely null, showing the loading row) until
          // the reply matching the CURRENT scope lands.
          if (env.scope !== s.ui.usageScope) return s
          // Drop a "session"-scope reply whose echoed session id no longer
          // matches the CURRENTLY attached session — the foreground session
          // switched while this request was in flight (scope stayed
          // "session" throughout), so this reply describes the OLD session
          // and must not render under the new attach.
          if (env.scope === 'session' && env.sessionId !== s.session.id) return s
          return {
            usagePreview: {
              cost: env.cost,
              tokensIn: env.tokensIn,
              tokensCached: env.tokensCached,
              tokensOut: env.tokensOut,
              calls: env.calls,
              days: env.days,
              topModels: env.topModels,
            },
            usagePreviewBusy: false,
          }
        })
        break
      case 'Analytics':
        set((s) => {
          // Reject a stale reply: every correlation input must still match the
          // CURRENT filters + the reqSeq that was in flight when this reply
          // was requested. Out-of-order / superseded replies leave the slice
          // alone (still loading for the newer request, or already settled).
          if (env.reqSeq !== s.analytics.reqSeq) return s
          if (env.scope !== s.analytics.scope) return s
          if (env.range !== s.analytics.range) return s
          if (env.metric !== s.analytics.metric) return s
          // Session-scope replies must still match the CURRENTLY attached
          // session (and the sessionId we actually requested).
          if (env.scope === 'session') {
            if (env.sessionId !== s.session.id) return s
            if (env.sessionId !== s.analytics.sessionId) return s
          }
          if (env.status === 'error') {
            return {
              analytics: {
                ...s.analytics,
                loading: false,
                error: env.error ?? 'Failed to load analytics',
                data: null,
                hasData: false,
              },
            }
          }
          // ok OR empty — both are successful replies. Empty still stores the
          // zeroed payload so the tab can render an empty state without a
          // permanent spinner.
          return {
            analytics: {
              ...s.analytics,
              loading: false,
              error: null,
              hasData: true,
              data: {
                cost: env.cost,
                tokensIn: env.tokensIn,
                tokensCached: env.tokensCached,
                tokensOut: env.tokensOut,
                calls: env.calls,
                cacheRate: env.cacheRate,
                series: env.series,
                models: env.models,
                mainCost: env.mainCost,
                mainCalls: env.mainCalls,
                subCost: env.subCost,
                subCalls: env.subCalls,
              },
            },
          }
        })
        break
    default:
      return false
  }
  return true
}
