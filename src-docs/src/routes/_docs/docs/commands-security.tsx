import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getSecuritySteps } from '../../../demos/security-tutorial'

export const Route = createFileRoute('/_docs/docs/commands-security')({
  component: CommandsSecurityPage,
})

function CommandsSecurityPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Command: /security</h1>
      <p className="mb-6 text-koma-fg">
        <code className="text-koma-fg">/security</code> opens the security daemon control panel —
        a full-screen status view for the optional Python security toolkit. The daemon is not part
        of the normal install: provision it once with{' '}
        <code className="text-koma-fg">koma --security-install</code> (requires Python 3.8+, and
        unsupported on Windows) before the panel can show a running daemon.
      </p>

      <TuiTutorial steps={getSecuritySteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Install command</strong>{' '}
          Run <code className="text-koma-fg">koma --security-install</code> from a normal shell (not
          inside Koma) first. It extracts the bundled assets into{' '}
          <code className="text-koma-fg">~/.koma/security/</code>, creates a Python venv, and
          installs the dependencies.
        </p>
        <p>
          <strong className="text-koma-accent">Per-launch daemon activation</strong>{' '}
          The daemon is NOT auto-started when Koma launches. Open{' '}
          <code className="text-koma-fg">/security</code> and press{' '}
          <code className="text-koma-fg">Space</code> on the{' '}
          <code className="text-koma-fg">[ ] Daemon running</code> row to start it each session.
        </p>
        <p>
          <strong className="text-koma-accent">Tool &amp; YOLO controls</strong>{' '}
          <code className="text-koma-fg">↑↓</code> move,{' '}
          <code className="text-koma-fg">Space</code>/
          <code className="text-koma-fg">Enter</code> toggles the selected row (daemon, a tool, or a
          whole domain via <code className="text-koma-fg">d</code>),{' '}
          <code className="text-koma-fg">h</code> switches to the dependency-health pane,{' '}
          <code className="text-koma-fg">i</code> installs the selected dependency,{' '}
          <code className="text-koma-fg">r</code> restarts,{' '}
          <code className="text-koma-fg">Esc</code> closes. “Enable YOLO mode” stays locked until the
          daemon is running.
        </p>
        <p>
          <strong className="text-koma-accent">Limitations</strong>{' '}
          Security mode requires Python 3.8+ and is unsupported on Windows. The panel shows the
          daemon lifecycle and tool inventory only — it does not expose safety-policy editing,
          alerts, or an audit log.
        </p>
      </div>
    </article>
  )
}
