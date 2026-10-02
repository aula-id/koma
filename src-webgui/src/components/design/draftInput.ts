import { useEffect, useRef, useState, type FocusEvent, type KeyboardEvent, type ChangeEvent } from 'react'
import { DESIGN_PATCH_DEBOUNCE_MS } from './tabShared'

export function formatBareNumber(value: number): string {
  if (!Number.isFinite(value)) return '0'
  const rounded = Math.round(value * 100) / 100
  return String(rounded)
}

export function isBareNumberDraft(raw: string): boolean {
  return raw === '' || /^-?\d*\.?\d*$/.test(raw)
}

export function parseCompleteNumber(raw: string): number | null {
  const trimmed = raw.trim()
  if (trimmed === '' || trimmed === '-' || trimmed === '.' || trimmed === '-.') return null
  if (!/^-?\d+\.?\d*$|^-?\d*\.\d+$/.test(trimmed)) return null
  const next = Number(trimmed)
  return Number.isFinite(next) ? next : null
}

export function parseBareNumber(raw: string, emptyAs = 0): number {
  return parseCompleteNumber(raw) ?? emptyAs
}

export function useDraftNumber({
  value,
  mixed = false,
  emptyAs = 0,
  onChange,
}: {
  value: number
  mixed?: boolean
  emptyAs?: number
  onChange: (value: number) => void
}) {
  const [draft, setDraft] = useState<string | null>(null)
  const focused = useRef(false)
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lastSent = useRef(value)
  const shown = mixed ? '' : draft ?? formatBareNumber(value)

  useEffect(() => {
    if (!focused.current) setDraft(null)
    lastSent.current = value
  }, [value, mixed])

  useEffect(() => () => {
    if (debounceRef.current) clearTimeout(debounceRef.current)
  }, [])

  const emit = (next: number) => {
    if (next === lastSent.current) return
    lastSent.current = next
    onChange(next)
  }

  const commit = (raw: string, allowEmpty: boolean) => {
    const parsed = parseCompleteNumber(raw)
    if (parsed != null) {
      emit(parsed)
      return
    }
    if (allowEmpty && raw.trim() === '') emit(emptyAs)
  }

  const clearDebounce = () => {
    if (!debounceRef.current) return
    clearTimeout(debounceRef.current)
    debounceRef.current = null
  }

  const onFocus = (event: FocusEvent<HTMLInputElement>) => {
    focused.current = true
    lastSent.current = value
    if (!mixed) setDraft(formatBareNumber(value))
    const target = event.currentTarget
    requestAnimationFrame(() => target.select())
    target.addEventListener('mouseup', (up) => up.preventDefault(), { once: true })
  }

  const onChangeDraft = (event: ChangeEvent<HTMLInputElement>) => {
    if (mixed) return
    const raw = event.target.value
    if (!isBareNumberDraft(raw)) return
    setDraft(raw)
    clearDebounce()
    if (parseCompleteNumber(raw) == null) return
    debounceRef.current = setTimeout(() => {
      debounceRef.current = null
      if (focused.current) commit(raw, false)
    }, DESIGN_PATCH_DEBOUNCE_MS)
  }

  const onBlur = () => {
    focused.current = false
    clearDebounce()
    if (!mixed) commit(draft ?? formatBareNumber(value), true)
    setDraft(null)
  }

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault()
      clearDebounce()
      commit(draft ?? formatBareNumber(value), true)
      event.currentTarget.blur()
      return
    }
    if (event.key === 'Escape') {
      event.preventDefault()
      clearDebounce()
      setDraft(null)
      focused.current = false
      event.currentTarget.blur()
      return
    }
    if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return
    event.preventDefault()
    const base = parseCompleteNumber(draft ?? '') ?? value
    const step = event.altKey ? 0.1 : event.shiftKey ? 10 : 1
    const next = event.key === 'ArrowUp' ? base + step : base - step
    setDraft(formatBareNumber(next))
    clearDebounce()
    emit(next)
  }

  return {
    shown,
    draft,
    setDraft,
    focused,
    inputProps: {
      type: 'text' as const,
      inputMode: 'decimal' as const,
      value: shown,
      placeholder: mixed ? 'Mixed' : undefined,
      onFocus,
      onChange: onChangeDraft,
      onBlur,
      onKeyDown,
    },
  }
}
