import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/docs/commands-store')({
  component: () => (
    <CommandPage
      name="/store"
      description="Browse and install extensions from the koma.run marketplace."
      details={
        <p>Opens the extension marketplace where you can discover, install, and update community extensions. Extensions add new tools, UI panels, and integrations to koma.</p>
      }
    />
  ),
})
