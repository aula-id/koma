import { Plus } from 'lucide-react'
import { AccordionSection } from '../AccordionSection'

function AddAction({ label }: { label: string }) {
  return (
    <button
      title={label}
      aria-label={label}
      className="flex h-5 w-5 items-center justify-center rounded text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
    >
      <Plus size={14} />
    </button>
  )
}

function Empty({ children }: { children: string }) {
  return <div className="px-5 py-1.5 text-[12px] text-koma-fg opacity-35">{children}</div>
}

// Design-phase stub. Sections mirror koma's credential catalogues (API-key
// providers, OAuth connections, models) but hold no data yet.
export function ConnectorPanel() {
  return (
    <>
      <AccordionSection title="Providers" action={<AddAction label="Add provider" />}>
        <Empty>No providers</Empty>
      </AccordionSection>
      <AccordionSection title="OAuth" action={<AddAction label="Connect account" />}>
        <Empty>No connections</Empty>
      </AccordionSection>
      <AccordionSection title="Models" action={<AddAction label="Add model" />}>
        <Empty>No models</Empty>
      </AccordionSection>
    </>
  )
}
