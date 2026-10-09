import { useEffect, useState } from 'react'
import { useKoma } from '../store/koma'
import { useNotifications, notificationRequest, scopeKey } from '../lib/notifications'

export default function NotificationsTab() {
  const session = useKoma(s => s.session.id)
  const [app, setApp] = useState(!session)
  const [query, setQuery] = useState('')
  const [severity, setSeverity] = useState('all')
  const [expanded, setExpanded] = useState<string | null>(null)
  const [confirm, setConfirm] = useState(false)
  useEffect(() => { if (!session) setApp(true) }, [session])
  const scope = app ? null : session
  const rows = useNotifications(s => s.scopes[scopeKey(scope)]) ?? []
  const error = useNotifications(s => s.errors[scopeKey(scope)])
  useEffect(() => {
    setExpanded(null); setConfirm(false)
    notificationRequest(scope, { op: 'list' })
    const timer = window.setInterval(() => notificationRequest(scope, { op: 'list' }), 3000)
    return () => window.clearInterval(timer)
  }, [scope])
  const filtered = rows.filter(e => (severity === 'all' || e.severity === severity) && `${e.message} ${e.source}`.toLowerCase().includes(query.toLowerCase()))
  return <div className="h-full overflow-auto bg-koma-bg p-5 text-koma-fg" data-tour="notification-history">
    <h2 className="mb-4 text-lg">Notifications <span aria-label="Unread notifications">({rows.filter(e => !e.read).length} unread)</span></h2>
    <div className="mb-4 flex flex-wrap gap-3">
      <select aria-label="Notification scope" value={app ? 'app' : 'session'} onChange={e => setApp(e.target.value === 'app')} className="bg-koma-panel2 p-2">
        <option value="session" disabled={!session}>Session</option><option value="app">App</option>
      </select>
      <input aria-label="Search notifications" placeholder="Search history" value={query} onChange={e => setQuery(e.target.value)} className="bg-koma-panel2 p-2" />
      <select aria-label="Severity" value={severity} onChange={e => setSeverity(e.target.value)} className="bg-koma-panel2 p-2">{['all', 'info', 'success', 'warn', 'error'].map(k => <option key={k}>{k}</option>)}</select>
      <button onClick={() => notificationRequest(scope, { op: 'read', id: null })}>Mark all read</button>
      <button onClick={() => setConfirm(true)}>Clear history</button>
    </div>
    {confirm && <div role="alertdialog" aria-label="Clear notification history" className="mb-4 border border-koma-border p-3">Clear {app ? 'App' : 'Session'} history permanently? <button onClick={() => { notificationRequest(scope, { op: 'clear' }); setConfirm(false) }}>Confirm clear</button> <button onClick={() => setConfirm(false)}>Cancel</button></div>}
    {error && <p role="alert">History unavailable: {error}. Temporary popups still work.</p>}
    {!error && !filtered.length && <p>No notifications.</p>}
    {filtered.map(e => <div key={e.id} className="border-b border-koma-border py-3">
      <button className="flex w-full gap-3 text-left" onClick={() => { setExpanded(expanded === e.id ? null : e.id); notificationRequest(scope, { op: 'read', id: e.id }) }} aria-expanded={expanded === e.id}>
        <span>{e.read ? '' : '●'}</span><span className="text-xs opacity-60">{e.severity} · {new Date(e.timestamp).toLocaleString()}</span><span className="min-w-0 flex-1 truncate">{e.message}</span>
      </button>
      {expanded === e.id && <div className="mt-2 whitespace-pre-wrap text-sm"><p>{e.message}</p><p className="mt-2 text-xs opacity-50">{e.source} · {e.id}</p></div>}
    </div>)}
  </div>
}
