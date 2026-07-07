import { useState } from 'react'
import { AccordionSection } from '../AccordionSection'
import { Empty } from './helpers'

export function ExplorePanel() {
  const [open, setOpen] = useState({ files: true, bash: true, agents: true })

  return (
    <div className="h-full overflow-auto">
      <AccordionSection
        title="File changed"
        open={open.files}
        onToggle={() => setOpen((s) => ({ ...s, files: !s.files }))}
      >
        <Empty>No changes</Empty>
      </AccordionSection>
      <AccordionSection
        title="Bash"
        open={open.bash}
        onToggle={() => setOpen((s) => ({ ...s, bash: !s.bash }))}
      >
        <Empty>No bash sessions</Empty>
      </AccordionSection>
      <AccordionSection
        title="Agents"
        open={open.agents}
        onToggle={() => setOpen((s) => ({ ...s, agents: !s.agents }))}
      >
        <Empty>No agents</Empty>
      </AccordionSection>
    </div>
  )
}
