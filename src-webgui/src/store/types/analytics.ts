export type AnalyticsScope = 'all' | 'session'
export type AnalyticsRange = 'today' | '7d' | '30d' | 'year'
export type AnalyticsMetric = 'cost' | 'tokens'
export type AnalyticsStatus = 'ok' | 'empty' | 'error'

// One time-series bucket in an Analytics reply.
export type AnalyticsSeriesPoint = {
  epoch: number
  cost: number
  tokens: number
}

// One model row in an Analytics reply (full token breakdown).
export type AnalyticsModelRow = {
  modelId: string
  cost: number
  tokensIn: number
  tokensCached: number
  tokensOut: number
  calls: number
}

// Authoritative Analytics dashboard payload (host `Analytics` reply body when
// status is ok/empty). Correlation fields are stored separately on the slice
// so the reducer can reject stale replies.
export type AnalyticsData = {
  cost: number
  tokensIn: number
  tokensCached: number
  tokensOut: number
  calls: number
  // cacheRate = tokensCached / (tokensIn + tokensCached), or 0 when denom is 0.
  cacheRate: number
  series: AnalyticsSeriesPoint[]
  models: AnalyticsModelRow[]
  mainCost: number
  mainCalls: number
  subCost: number
  subCalls: number
}
