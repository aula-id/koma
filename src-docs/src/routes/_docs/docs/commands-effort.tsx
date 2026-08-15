import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/docs/commands-effort')({
  component: () => (
    <CommandPage
      name="/effort"
      description="Set the model's reasoning/thinking effort level."
      details={
        <p>Controls how much thinking the model does before responding. Lower effort = faster but less thorough. Higher effort = more careful but slower. Useful for balancing speed vs. quality on different task types.</p>
      }
    />
  ),
})
