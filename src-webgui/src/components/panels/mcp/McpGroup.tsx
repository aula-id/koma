import { cloneElement, isValidElement, useId, type ReactNode } from 'react'
import { ChevronRight } from 'lucide-react'
import { HelpTip } from '../helpers'

/** Label above the control, optional (?) help beside it, one field per row. */
export function FormRow({
  label,
  help,
  error,
  htmlFor,
  children,
}: {
  label: string
  help?: ReactNode
  error?: string
  htmlFor?: string
  children: ReactNode
}) {
  const uid = useId()
  const labelId = `${uid}-label`
  const errorId = error ? `${uid}-error` : undefined
  const describedBy = errorId
  // A real input gets the error text as its description; a control
  // without an id (the segmented choice) is grouped under the label instead.
  const control =
    htmlFor && isValidElement<Record<string, unknown>>(children)
      ? cloneElement(children, { 'aria-describedby': describedBy, 'aria-invalid': error ? true : undefined })
      : children
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <div className="flex min-w-0 items-center gap-1">
        {htmlFor ? (
          <label htmlFor={htmlFor} className="text-[11px] font-semibold text-koma-fg opacity-75">
            {label}
          </label>
        ) : (
          <span id={labelId} className="text-[11px] font-semibold text-koma-fg opacity-75">
            {label}
          </span>
        )}
        {help && <HelpTip label={label}>{help}</HelpTip>}
      </div>
      {htmlFor ? (
        control
      ) : (
        <div role="group" aria-labelledby={labelId} aria-describedby={describedBy}>
          {control}
        </div>
      )}
      {error && (
        <span id={errorId} role="alert" className="text-[11px] text-koma-error">
          {error}
        </span>
      )}
    </div>
  )
}

/** Collapsible group: title plus a one-line summary so set values are visible when closed. */
export function FormGroup({
  title,
  help,
  summary,
  open,
  onToggle,
  children,
}: {
  title: string
  help?: ReactNode
  summary: string
  open: boolean
  onToggle: () => void
  children: ReactNode
}) {
  const bodyId = useId()
  return (
    <section className="rounded border border-koma-border">
      <div className="flex min-w-0 items-center pr-2">
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={open}
          aria-controls={open ? bodyId : undefined}
          className="flex min-w-0 flex-1 items-center gap-1.5 px-2 py-2 text-left hover:bg-koma-hover"
        >
          <ChevronRight
            size={14}
            strokeWidth={2}
            className={`flex-none text-koma-fg opacity-70 transition-transform ${open ? 'rotate-90' : ''}`}
          />
          <span className="flex min-w-0 flex-col">
            <span className="text-[12px] font-semibold leading-snug text-koma-fg">{title}</span>
            <span className="truncate text-[11px] text-koma-fg opacity-60">{summary}</span>
          </span>
        </button>
        {help && <HelpTip label={title}>{help}</HelpTip>}
      </div>
      {open && (
        <div id={bodyId} className="flex min-w-0 flex-col gap-4 border-t border-koma-border px-2 py-3">
          {children}
        </div>
      )}
    </section>
  )
}

/** Sticky bottom bar of the MCP tabs; its content lines up with the page column. */
export function McpFooter({
  onCancel,
  onSave,
  saveDisabled,
  saveLabel = 'Save',
}: {
  onCancel: () => void
  onSave: () => void
  saveDisabled?: boolean
  saveLabel?: string
}) {
  return (
    <div className="flex-none border-t border-koma-border bg-koma-bg px-6 py-2.5" data-tour="form-actions">
      <div className="mx-auto flex max-w-3xl items-center justify-end gap-2">
        <button
          type="button"
          onClick={onCancel}
          data-tour="form-cancel"
          className="rounded px-3 py-1.5 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
        >
          Cancel
        </button>
        <button
          type="button"
          onClick={onSave}
          disabled={saveDisabled}
          data-tour="form-save"
          className="rounded border border-koma-border bg-koma-hover px-3.5 py-1.5 text-[12px] font-medium text-koma-fg transition-colors enabled:hover:bg-koma-head disabled:opacity-40"
        >
          {saveLabel}
        </button>
      </div>
    </div>
  )
}
