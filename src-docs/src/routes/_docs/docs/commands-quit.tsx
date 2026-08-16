import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getQuitSteps } from '../../../demos/quit-tutorial'

export const Route = createFileRoute('/_docs/docs/commands-quit')({
  component: CommandsQuitPage,
})

function CommandsQuitPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Command: /quit</h1>
      <p className="mb-6 text-koma-fg">
        <code className="text-koma-fg">/quit</code> (aliases{' '}
        <code className="text-koma-fg">/q</code>, <code className="text-koma-fg">/exit</code>) and the
        global <code className="text-koma-fg">Ctrl+C</code> keybind both route through the same quit
        chokepoint. koma always asks before quitting so you can choose to detach a session instead of
        killing it.
      </p>

      <TuiTutorial steps={getQuitSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Entry points</strong>{' '}
          <code className="text-koma-fg">Ctrl+C</code> is captured in every mode and behaves exactly
          like <code className="text-koma-fg">/quit</code> — there is no separate “cancel” meaning for
          it. The aliases <code className="text-koma-fg">/q</code> and{' '}
          <code className="text-koma-fg">/exit</code> also trigger the same path.
        </p>
        <p>
          <strong className="text-koma-accent">Always confirms</strong>{' '}
          The confirm overlay opens even when nothing is working, so you can detach idle sessions.
          Focus defaults to <code className="text-koma-fg">cancel</code> — the safe choice — so an
          accidental Enter will not close anything.
        </p>
        <p>
          <strong className="text-koma-accent">Three choices</strong>{' '}
          <code className="text-koma-fg">k</code> / quit closes the window (the session ends but stays
          on disk, reloadable from the session hub); <code className="text-koma-fg">d</code> / detach
          leaves the agent running headless and resumable from the hub’s history;{' '}
          <code className="text-koma-fg">Esc</code> / Enter cancels and returns to chat.
        </p>
        <p>
          <strong className="text-koma-accent">Immediate quit</strong>{' '}
          The overlay is skipped only on a landing/unconfigured screen (first-run chooser, provider
          wizard, or empty session list) — there, a quit request exits cleanly without opening the
          dialog.
        </p>
      </div>
    </article>
  )
}
