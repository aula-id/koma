/** Request provenance contains only scope IDs, never request bodies or credentials. */
export type Origin = { session: string | null; source: string }
const correlated = new Map<string, Origin>()
const queues = new Map<string, Origin[]>()
export let publishingOrigin: Origin | undefined
const APP_REQUESTS = /^(Key|SetProvider|DeleteProvider|StartOAuth|SubmitOAuth|CancelOAuth|DeleteOAuth|SetWebSearch|SetExtraSkillRoots|InstallExtension|UninstallExtension)/
const APP_REPLIES = /^(Key|OAuth|ExtensionOp)/
function correlation(v: Record<string, unknown>) {
  for (const key of ['requestId', 'reqSeq', 'req_seq']) if (v[key] !== undefined && v[key] !== 0 && v[key] !== '') return `${key}:${v[key]}`
  return null
}
function replyFamily(r: string) {
  if (r.startsWith('Git')) return `GitOp:${r.slice(3).replace(/^./, c => c.toLowerCase())}`
  if (r.startsWith('Key')) return 'KeyOp'
  if (r === 'SetAgent' || r === 'DeleteAgent') return 'AgentOp'
  return r
}
export function captureNotificationOrigin(request: Record<string, unknown>, session: string | null) {
  const r = String(request.r)
  if (r === 'Notifications') return
  const app = APP_REQUESTS.test(r) || (['SetModel', 'DeleteModel', 'SetAgent', 'DeleteAgent'].includes(r) && request.scope === 'global')
  const origin = { session: app ? null : session, source: app ? 'app' : r.startsWith('Git') ? 'git' : 'gui' }
  const key = correlation(request)
  if (key) {
    correlated.set(key, origin)
    if (correlated.size > 2000) correlated.delete(correlated.keys().next().value!)
  } else {
    const family = replyFamily(r)
    if (family.startsWith('GitOp:') || family === 'KeyOp') queues.set(family, [...(queues.get(family) ?? []).slice(-99), origin])
  }
}
export function withNotificationOrigin<T>(reply: Record<string, unknown>, currentSession: string | null, apply: () => T): T {
  const prior = publishingOrigin
  const key = correlation(reply)
  const family = reply.k === 'GitOp' ? `GitOp:${reply.op}` : String(reply.k)
  const saved = key ? correlated.get(key) : queues.get(family)?.shift()
  publishingOrigin = saved ?? { session: APP_REPLIES.test(String(reply.k)) ? null : typeof reply.session === 'string' ? reply.session : currentSession, source: APP_REPLIES.test(String(reply.k)) ? 'app' : 'gui' }
  try { return apply() } finally { publishingOrigin = prior }
}

export function notificationOrigin(requestId: string | undefined) { return requestId ? correlated.get(`requestId:${requestId}`) : undefined }
