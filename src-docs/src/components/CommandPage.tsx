import type { ReactNode } from 'react'

interface CommandPageProps {
  /** The slash command name, e.g. "/task" */
  name: string
  /** One-line description */
  description: string
  /** What the command does — paragraph(s) */
  details: ReactNode
  /** Keyboard shortcut alternative, if any */
  shortcut?: string
  /** Sub-commands or variants */
  variants?: { label: string; desc: string }[]
  /** Whether this command has a full step-by-step TUI tutorial */
  hasTutorial?: boolean
}

export function CommandPage({
  name,
  description,
  details,
  shortcut,
  variants,
}: CommandPageProps) {
  return (
    <article>
      <h1 className="mb-2 text-2xl font-bold text-koma-accent">Command: {name}</h1>
      <p className="mb-6 text-koma-fg">{description}</p>

      {shortcut && (
        <div className="mb-6 rounded-md border border-koma-border bg-koma-panel px-4 py-3 text-sm">
          <span className="text-koma-dim">Keyboard shortcut: </span>
          <code className="font-semibold text-koma-accent">{shortcut}</code>
        </div>
      )}

      <div className="space-y-4 text-sm leading-relaxed text-koma-dim">
        {details}
      </div>

      {variants && variants.length > 0 && (
        <div className="mt-8">
          <h3 className="mb-3 text-base font-semibold text-koma-fg">Variants</h3>
          <div className="space-y-2">
            {variants.map((v) => (
              <div
                key={v.label}
                className="flex items-start gap-3 rounded-md border border-koma-border bg-koma-panel px-4 py-2.5"
              >
                <code className="flex-none text-koma-accent">{v.label}</code>
                <span className="text-sm text-koma-dim">{v.desc}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </article>
  )
}
