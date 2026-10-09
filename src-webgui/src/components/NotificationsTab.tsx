import { useEffect, useState } from 'react'
import { AlertTriangle, CheckCircle2, Info, OctagonAlert, Search, TriangleAlert, type LucideIcon } from 'lucide-react'
import { useKoma } from '../store/koma'
import { useNotifications, notificationRequest, scopeKey, type NotificationEntry } from '../lib/notifications'
import { Segmented } from './panels/form'

const SEVERITY: Record<NotificationEntry['severity'], { Icon: LucideIcon; tone: string }> = {
  info: { Icon: Info, tone: 'text-koma-info' },
  success: { Icon: CheckCircle2, tone: 'text-koma-success' },
  warn: { Icon: TriangleAlert, tone: 'text-koma-warn' },
  error: { Icon: OctagonAlert, tone: 'text-koma-error' },
}

export default function NotificationsTab() {
  const session = useKoma((s) => s.session.id)
  const [app, setApp] = useState(!session)
  const [query, setQuery] = useState('')
  const [severity, setSeverity] = useState('all')
  const [expanded, setExpanded] = useState<string | null>(null)
  const [confirm, setConfirm] = useState(false)
  useEffect(() => {
    if (!session) setApp(true)
  }, [session])
  const scope = app ? null : session
  const rows = useNotifications((s) => s.scopes[scopeKey(scope)]) ?? []
  const error = useNotifications((s) => s.errors[scopeKey(scope)])
  useEffect(() => {
    setExpanded(null)
    setConfirm(false)
    notificationRequest(scope, { op: 'list' })
    const timer = window.setInterval(() => notificationRequest(scope, { op: 'list' }), 3000)
    return () => window.clearInterval(timer)
  }, [scope])
  useEffect(() => {
    if (!confirm) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setConfirm(false)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [confirm])
  const filtered = rows.filter(
    (e) =>
      (severity === 'all' || e.severity === severity) &&
      `${e.message} ${e.source}`.toLowerCase().includes(query.toLowerCase()),
  )
  const unread = rows.filter((e) => !e.read).length
  return (
    <div className="flex h-full min-h-0 flex-col bg-koma-bg text-[12px] leading-snug text-koma-fg" data-tour="notification-history">
      <header className="flex-none space-y-3 border-b border-koma-border px-4 pt-4 pb-3">
        <h2 className="flex items-baseline gap-2 text-[15px] font-semibold">
          Notifications
          <span className="text-[11px] font-normal text-koma-dim" aria-label="Unread notifications">
            ({unread} unread)
          </span>
        </h2>
        <div className="flex flex-wrap items-center gap-2">
          <div role="group" aria-label="Notification scope">
            <Segmented
              value={app ? 'app' : 'session'}
              options={[
                { value: 'session', label: 'Session', disabled: !session },
                { value: 'app', label: 'App' },
              ]}
              onChange={(v) => setApp(v === 'app')}
            />
          </div>
          <label className="relative min-w-[10rem] flex-1">
            <Search size={12} className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-koma-dim" aria-hidden />
            <input
              aria-label="Search notifications"
              placeholder="Search history"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              className="h-7 w-full rounded border border-koma-border bg-koma-panel2 py-0 pr-2 pl-7 text-[12px] text-koma-fg outline-none placeholder:text-koma-dim focus:border-koma-accent"
            />
          </label>
          <div role="group" aria-label="Severity">
            <Segmented
              value={severity}
              options={['all', 'info', 'success', 'warn', 'error'].map((k) => ({ value: k, label: k }))}
              onChange={setSeverity}
            />
          </div>
          <button
            type="button"
            className="rounded px-2 py-1 text-[11px] text-koma-dim transition-colors hover:bg-koma-hover hover:text-koma-fg"
            onClick={() => notificationRequest(scope, { op: 'read', id: null })}
          >
            Mark all read
          </button>
          <button
            type="button"
            className="rounded px-2 py-1 text-[11px] text-koma-dim transition-colors hover:bg-koma-hover hover:text-koma-fg"
            onClick={() => setConfirm(true)}
          >
            Clear history
          </button>
        </div>
        {confirm && (
          <div
            role="alertdialog"
            aria-label="Clear notification history"
            className="flex flex-wrap items-center gap-2 rounded-md border border-koma-error/40 bg-koma-error/10 px-3 py-2"
          >
            <AlertTriangle size={14} className="flex-none text-koma-error" aria-hidden />
            <span className="min-w-0 flex-1 text-[12px] text-koma-fg">
              Clear {app ? 'App' : 'Session'} history permanently?
            </span>
            <button
              type="button"
              autoFocus
              className="rounded bg-koma-error/15 px-2 py-1 text-[11px] font-semibold text-koma-error transition-colors hover:bg-koma-error/25"
              onClick={() => {
                notificationRequest(scope, { op: 'clear' })
                setConfirm(false)
              }}
            >
              Confirm clear
            </button>
            <button
              type="button"
              className="rounded px-2 py-1 text-[11px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
              onClick={() => setConfirm(false)}
            >
              Cancel
            </button>
          </div>
        )}
        {error && (
          <p role="alert" className="text-[12px] text-koma-error">
            History unavailable: {error}. Temporary popups still work.
          </p>
        )}
      </header>
      <div className="min-h-0 flex-1 overflow-auto">
        {!error && !filtered.length && <p className="px-4 py-6 text-koma-dim">No notifications.</p>}
        {filtered.map((e) => {
          const meta = SEVERITY[e.severity]
          const Icon = meta.Icon
          const open = expanded === e.id
          return (
            <div key={e.id} className={`border-b border-koma-border ${open ? 'bg-koma-panel2/60' : ''}`}>
              <button
                type="button"
                className={`flex w-full items-center gap-2 px-4 py-2 text-left text-[12px] transition-colors hover:bg-koma-hover ${e.read ? 'text-koma-dim' : 'text-koma-fg'}`}
                onClick={() => {
                  setExpanded(open ? null : e.id)
                  notificationRequest(scope, { op: 'read', id: e.id })
                }}
                aria-expanded={open}
              >
                <span
                  className={`h-1.5 w-1.5 flex-none rounded-full ${e.read ? 'bg-transparent' : 'bg-koma-accent'}`}
                  aria-hidden
                />
                <Icon size={13} className={`flex-none ${meta.tone}`} aria-hidden />
                <span className="min-w-0 flex-1 truncate">{e.message}</span>
                <span className={`shrink-0 text-[11px] ${meta.tone} opacity-80`}>{e.severity}</span>
                <span className="hidden shrink-0 text-[11px] text-koma-dim sm:inline">
                  {new Date(e.timestamp).toLocaleString()}
                </span>
              </button>
              {open && (
                <div className="space-y-1 px-4 pt-0 pb-3 pl-10 text-[12px] leading-relaxed">
                  <p className="whitespace-pre-wrap break-words text-koma-fg">{e.message}</p>
                  <p className="text-[11px] text-koma-dim">
                    {new Date(e.timestamp).toLocaleString()} · {e.source} · {e.id}
                  </p>
                </div>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}
