import { codingRequest } from './coding-service'
import { create } from 'zustand'
import { useKoma } from '../store/koma'
export type NotificationEntry = { id: string; timestamp: number; severity: 'info' | 'success' | 'warn' | 'error'; source: string; message: string; read: boolean }
export type NotificationOperation = { op: 'list' } | { op: 'record'; entry: NotificationEntry } | { op: 'read'; id: string | null } | { op: 'clear' }
export type NotificationRequest = { id: string; session: string | null; operation: NotificationOperation }
export type NotificationReply = { id: string; session: string | null; entries: NotificationEntry[]; error: string | null }
export const useNotifications = create<{ scopes: Record<string, NotificationEntry[]>; errors: Record<string, string | null> }>(() => ({ scopes: {}, errors: {} }))
const sessionHosts = new Map<string, string>()
export function bindNotificationHost(session: string | null, host: string | null) { if (session && !sessionHosts.has(session)) sessionHosts.set(session, host ?? 'local') }
export const scopeKey = (session: string | null) => session ?? 'app'
export function notificationRequest(session: string | null, operation: NotificationOperation) {
  const state = useKoma.getState()
  if (session && !sessionHosts.has(session) && state.session.id === session) bindNotificationHost(session, state.remoteState.hostId)
  const request: NotificationRequest = { id: crypto.randomUUID(), session, operation }
  const hostId = session ? sessionHosts.get(session) ?? 'local' : 'local'
  if (hostId !== 'local') {
    // Retained host RPC remains bound to the origin after chat attachment changes.
    // No workspace path is used to resolve notification storage.
    void codingRequest<NotificationReply>({ hostId, root: '' }, { op: 'notifications', request }).then(receiveNotifications).catch(error => {
      console.error('Notification history:', error)
      receiveNotifications({ id: request.id, session, entries: [], error: String(error) })
    })
  } else if (session && state.remoteState.hostId) {
    // The active daemon may be remote; an old local scope belongs to the host.
    state.req({ r: 'Notifications', request, local: true })
  } else state.req({ r: 'Notifications', request })
}

export function receiveNotifications(reply: NotificationReply) {
  const key = scopeKey(reply.session)
  useNotifications.setState(s => ({ scopes: { ...s.scopes, ...(reply.error ? {} : { [key]: reply.entries }) }, errors: { ...s.errors, [key]: reply.error } }))
}
/** Shared publisher used by every GUI toast, including legacy store producers. */
export function publishNotification(message: string, severity: NotificationEntry['severity'], session: string | null, source = 'gui', eventId?: string) {
  const entry: NotificationEntry = { id: eventId ?? crypto.randomUUID(), timestamp: Date.now(), severity, source, message, read: false }
  notificationRequest(session, { op: 'record', entry })
  return entry
}
