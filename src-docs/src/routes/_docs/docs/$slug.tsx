import { createFileRoute, Navigate } from '@tanstack/react-router'

/** Legacy redirect mapper: /docs/:slug → new location. */
const REDIRECT_MAP: Record<string, string> = {
  // Welcome pages
  'overview': '/welcome',
  'getting-started': '/welcome/getting-started',
  'architecture': '/welcome/architecture',

  // TUI top-level
  'tui': '/tui',
  'tutorial-first-run': '/tui/first-run',
  'tutorial-provider-model': '/tui/provider-model',
  'tutorial-oauth': '/tui/oauth',
  'keyboard-shortcuts': '/tui/keyboard-shortcuts',
  'commands-all': '/tui/commands-all',

  // TUI settings
  'settings-appearance': '/tui/settings-appearance',
  'settings-general': '/tui/settings-general',
  'settings-provider': '/tui/settings-provider',
  'settings-oauth': '/tui/settings-oauth',
  'settings-model': '/tui/settings-model',

  // GUI
  'gui': '/gui',
  'tutorial-gui-first-run': '/gui/first-run',
}

function LegacyDocsRedirect() {
  const { slug } = Route.useParams()

  // Check explicit map first
  const target = REDIRECT_MAP[slug]
  if (target) return <Navigate to={target} />

  // Commands: /docs/commands-foo → /tui/commands-foo
  if (slug.startsWith('commands-')) {
    return <Navigate to={`/tui/${slug}`} />
  }

  // Unknown legacy slug → welcome
  return <Navigate to="/welcome" />
}

export const Route = createFileRoute('/_docs/docs/$slug')({
  component: LegacyDocsRedirect,
})
