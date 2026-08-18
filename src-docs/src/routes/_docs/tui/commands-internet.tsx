import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getInternetSteps } from '../../../demos/internet-tutorial'

export const Route = createFileRoute('/_docs/tui/commands-internet')({
  component: CommandsInternetPage,
})

function CommandsInternetPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Command: /internet</h1>
      <p className="mb-6 text-koma-fg">
        <code className="text-koma-fg">/internet</code> toggles the agent's internet mode between{' '}
        <strong className="text-koma-accent">simple</strong> and{' '}
        <strong className="text-koma-accent">full</strong>. Simple keeps{' '}
        <code className="text-koma-fg">web_search</code>/
        <code className="text-koma-fg">web_fetch</code>; full unlocks the browser-backed tools
        (rendered pages, Cloudflare bypass) after you provision the Firefox-for-Playwright backend
        with <code className="text-koma-fg">koma --internet-fullmode-install</code>.
      </p>

      <TuiTutorial steps={getInternetSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Variants</strong>{' '}
          <code className="text-koma-fg">/internet</code> toggles;{' '}
          <code className="text-koma-fg">/internet simple</code> and{' '}
          <code className="text-koma-fg">/internet full</code> set a named mode explicitly.{' '}
          <code className="text-koma-fg">Ctrl+E</code> toggles the same setting in the chat.
        </p>
        <p>
          <strong className="text-koma-accent">Prerequisite installer</strong>{' '}
          Full mode needs the browser backend, installed once from a shell with{' '}
          <code className="text-koma-fg">koma --internet-fullmode-install</code> (downloads ~80 MB of
          Firefox for Playwright into <code className="text-koma-fg">~/.koma/internet/</code>).
        </p>
        <p>
          <strong className="text-koma-accent">Simple vs Full boundary</strong>{' '}
          The full-mode tools are advertised in both modes. In simple mode calling one returns an
          install/mode error rather than being silently hidden; switching to full activates them.
        </p>
        <p>
          <strong className="text-koma-accent">Persistence</strong>{' '}
          The chosen mode persists in the session. Selecting{' '}
          <code className="text-koma-fg">full</code> without the backend flashes{' '}
          <code className="text-koma-fg">internet: full needs `koma --internet-fullmode-install`</code>{' '}
          instead of enabling the browser tools.
        </p>
      </div>
    </article>
  )
}
