import { AccordionSection } from '../AccordionSection'

function Empty({ children }: { children: string }) {
  return <div className="px-5 py-1.5 text-[12px] text-koma-fg opacity-35">{children}</div>
}

export function ExplorePanel() {
  return (
    <>
      <AccordionSection title="File changed">
        <Empty>No changes</Empty>
      </AccordionSection>
      <AccordionSection title="Bash">
        <Empty>No bash sessions</Empty>
      </AccordionSection>
      <AccordionSection title="Agents">
        <Empty>No agents</Empty>
      </AccordionSection>
    </>
  )
}
