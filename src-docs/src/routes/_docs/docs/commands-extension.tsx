import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/docs/commands-extension')({
  component: () => (
    <CommandPage
      name="/extension"
      description="Manage installed extensions (detail, uninstall, screens)."
      details={
        <p>Opens the extensions management panel. View installed extensions, check their status, uninstall them, or view their screens and configuration.</p>
      }
    />
  ),
})
