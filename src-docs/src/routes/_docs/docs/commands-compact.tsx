import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/docs/commands-compact')({
  component: () => (
    <CommandPage
      name="/compact"
      description="Summarize and compact the conversation to free up context window space."
      details={
        <p>Triggers context compaction — the agent summarizes the conversation so far, replacing the full transcript with a compressed summary. This frees context window space for longer sessions without losing continuity.</p>
      }
    />
  ),
})
