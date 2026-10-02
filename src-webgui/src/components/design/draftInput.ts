import { useEffect, useRef, useState, type ChangeEvent, type FocusEvent, type KeyboardEvent, type PointerEvent, type WheelEvent } from 'react'
import { DESIGN_PATCH_DEBOUNCE_MS } from './tabShared'

export function formatBareNumber(value: number): string {
  if (!Number.isFinite(value)) return '0'
  const rounded = Math.round(value * 100) / 100
  return String(rounded)
}

export function isExprDraft(raw: string): boolean {
  return raw === '' || /^[\s0-9+\-*/().,%]*$/.test(raw)
}

export function parseCompleteNumber(raw: string): number | null {
  return evalNumberExpr(raw, 0)
}

export function parseBareNumber(raw: string, emptyAs = 0): number {
  return evalNumberExpr(raw, emptyAs) ?? emptyAs
}

/** Penpot simple-math: `12+4`, `(2+3)*4`, `+10` / `*2` / `/2` relative to last, `10%` of last. */
export function evalNumberExpr(raw: string, last = 0): number | null {
  const text = raw.trim().replace(/,/g, '.').replace(/\s+/g, '').replace(/\.$/, '')
  if (!text) return null
  if (/^[+*/]/.test(text)) {
    const rest = evalNumberExpr(text.slice(1), last)
    if (rest == null) return null
    if (text[0] === '+') return last + rest
    if (text[0] === '*') return last * rest
    return rest === 0 ? null : last / rest
  }
  const tokens = tokenizeExpr(text)
  if (!tokens) return null
  let index = 0
  const peek = () => tokens[index]
  const eat = () => tokens[index++]
  const parseFactor = (): number | null => {
    const token = peek()
    if (!token) return null
    if (token.kind === '-') {
      eat()
      const inner = parseFactor()
      return inner == null ? null : -inner
    }
    if (token.kind === '(') {
      eat()
      const inner = parseSum()
      if (peek()?.kind !== ')') return null
      eat()
      return inner
    }
    if (token.kind === 'number') {
      eat()
      return token.percent ? (token.value / 100) * last : token.value
    }
    return null
  }
  const parseProduct = (): number | null => {
    let value = parseFactor()
    if (value == null) return null
    while (peek()?.kind === '*' || peek()?.kind === '/') {
      const op = eat()
      const next = parseFactor()
      if (next == null) return null
      if (op.kind === '*') value *= next
      else {
        if (next === 0) return null
        value /= next
      }
    }
    return value
  }
  const parseSum = (): number | null => {
    let value = parseProduct()
    if (value == null) return null
    while (peek()?.kind === '+' || peek()?.kind === '-') {
      const op = eat()
      const next = parseProduct()
      if (next == null) return null
      value = op.kind === '+' ? value + next : value - next
    }
    return value
  }
  const value = parseSum()
  if (value == null || index !== tokens.length || !Number.isFinite(value)) return null
  return value
}

type ExprToken =
  | { kind: 'number'; value: number; percent: boolean }
  | { kind: '+' | '-' | '*' | '/' | '(' | ')' }

function tokenizeExpr(text: string): ExprToken[] | null {
  const tokens: ExprToken[] = []
  let i = 0
  while (i < text.length) {
    const char = text[i]
    if (char === '+' || char === '-' || char === '*' || char === '/' || char === '(' || char === ')') {
      tokens.push({ kind: char })
      i += 1
      continue
    }
    const match = text.slice(i).match(/^[0-9]*\.?[0-9]+%?/)
    if (!match) return null
    const raw = match[0]
    const percent = raw.endsWith('%')
    const value = Number(percent ? raw.slice(0, -1) : raw)
    if (!Number.isFinite(value)) return null
    tokens.push({ kind: 'number', value, percent })
    i += raw.length
  }
  return tokens
}

function clampNumber(value: number, min?: number, max?: number): number {
  let next = value
  if (min != null) next = Math.max(min, next)
  if (max != null) next = Math.min(max, next)
  return next
}

function nudgeStep(event: { altKey: boolean; shiftKey: boolean; ctrlKey?: boolean }, step: number): number {
  if (event.shiftKey) return step * 10
  if (event.altKey || event.ctrlKey) return step * 0.1
  return step
}

export function useDraftNumber({
  value,
  mixed = false,
  emptyAs = 0,
  nillable = false,
  min,
  max,
  step = 1,
  live = true,
  onChange,
  onClear,
  onTokenShortcut,
}: {
  value: number
  mixed?: boolean
  emptyAs?: number
  nillable?: boolean
  min?: number
  max?: number
  step?: number
  live?: boolean
  onChange: (value: number) => void
  onClear?: () => void
  onTokenShortcut?: () => void
}) {
  const [draft, setDraft] = useState<string | null>(null)
  const focused = useRef(false)
  const dirty = useRef(false)
  const rawRef = useRef(formatBareNumber(value))
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lastSent = useRef(value)
  const lastGood = useRef(value)
  const scrub = useRef<{ x: number; start: number; dragging: boolean } | null>(null)
  const commitRef = useRef<(restore: boolean) => void>(() => {})
  const shown = mixed ? '' : draft ?? formatBareNumber(value)

  useEffect(() => {
    if (!focused.current) setDraft(null)
    lastSent.current = value
    lastGood.current = value
    if (!dirty.current) rawRef.current = formatBareNumber(value)
  }, [value, mixed])

  useEffect(() => () => {
    if (debounceRef.current) clearTimeout(debounceRef.current)
    commitRef.current(false)
  }, [])

  const emit = (next: number) => {
    const clamped = clampNumber(next, min, max)
    if (clamped === lastSent.current) return
    lastSent.current = clamped
    lastGood.current = clamped
    onChange(clamped)
  }

  const applyRaw = (raw: string, allowEmpty: boolean, restoreInvalid: boolean, formatDraft = true) => {
    const parsed = evalNumberExpr(raw, lastGood.current)
    if (parsed != null) {
      emit(parsed)
      if (formatDraft) {
        setDraft(formatBareNumber(clampNumber(parsed, min, max)))
        dirty.current = false
      }
      return true
    }
    if (allowEmpty && raw.trim() === '') {
      if (nillable) {
        lastSent.current = emptyAs
        onClear?.()
      } else {
        emit(emptyAs)
      }
      dirty.current = false
      return true
    }
    if (restoreInvalid) {
      setDraft(formatBareNumber(lastGood.current))
      dirty.current = false
    }
    return false
  }

  commitRef.current = (restore) => {
    if (dirty.current) applyRaw(rawRef.current, true, restore)
  }

  const clearDebounce = () => {
    if (!debounceRef.current) return
    clearTimeout(debounceRef.current)
    debounceRef.current = null
  }

  const onFocus = (event: FocusEvent<HTMLInputElement>) => {
    focused.current = true
    lastSent.current = value
    lastGood.current = value
    if (!mixed) {
      const text = formatBareNumber(value)
      setDraft(text)
      rawRef.current = text
    }
    const target = event.currentTarget
    requestAnimationFrame(() => target.select())
    target.addEventListener('mouseup', (up) => up.preventDefault(), { once: true })
  }

  const onChangeDraft = (event: ChangeEvent<HTMLInputElement>) => {
    if (mixed) return
    const raw = event.target.value
    if (!isExprDraft(raw)) return
    setDraft(raw)
    rawRef.current = raw
    dirty.current = true
    clearDebounce()
    if (!live || evalNumberExpr(raw, lastGood.current) == null) return
    debounceRef.current = setTimeout(() => {
      debounceRef.current = null
      if (focused.current) applyRaw(raw, false, false, false)
    }, DESIGN_PATCH_DEBOUNCE_MS)
  }

  const finish = (restoreInvalid: boolean) => {
    focused.current = false
    clearDebounce()
    if (!mixed) applyRaw(draft ?? rawRef.current ?? formatBareNumber(value), true, restoreInvalid)
    setDraft(null)
    dirty.current = false
  }

  const onBlur = () => finish(true)

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === '{' && onTokenShortcut) {
      event.preventDefault()
      onTokenShortcut()
      return
    }
    if (event.key === 'Enter') {
      event.preventDefault()
      clearDebounce()
      applyRaw(draft ?? formatBareNumber(value), true, true)
      event.currentTarget.blur()
      return
    }
    if (event.key === 'Escape') {
      event.preventDefault()
      clearDebounce()
      setDraft(formatBareNumber(lastGood.current))
      rawRef.current = formatBareNumber(lastGood.current)
      dirty.current = false
      focused.current = false
      event.currentTarget.blur()
      return
    }
    if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return
    event.preventDefault()
    const base = evalNumberExpr(draft ?? '', lastGood.current) ?? lastGood.current
    const next = clampNumber(event.key === 'ArrowUp' ? base + nudgeStep(event, step) : base - nudgeStep(event, step), min, max)
    setDraft(formatBareNumber(next))
    rawRef.current = formatBareNumber(next)
    dirty.current = false
    clearDebounce()
    emit(next)
  }

  const onWheel = (event: WheelEvent<HTMLInputElement>) => {
    if (!focused.current) return
    event.preventDefault()
    const base = evalNumberExpr(draft ?? '', lastGood.current) ?? lastGood.current
    const next = clampNumber(event.deltaY < 0 ? base + step : base - step, min, max)
    setDraft(formatBareNumber(next))
    rawRef.current = formatBareNumber(next)
    dirty.current = false
    emit(next)
  }

  const onScrubPointerDown = (event: PointerEvent<HTMLElement>) => {
    if (mixed || event.button !== 0) return
    event.preventDefault()
    scrub.current = { x: event.clientX, start: lastGood.current, dragging: false }
    event.currentTarget.setPointerCapture(event.pointerId)
  }

  const onScrubPointerMove = (event: PointerEvent<HTMLElement>) => {
    const state = scrub.current
    if (!state) return
    const delta = event.clientX - state.x
    if (!state.dragging && Math.abs(delta) < 3) return
    state.dragging = true
    const next = clampNumber(state.start + Math.round(delta) * nudgeStep(event, step), min, max)
    setDraft(formatBareNumber(next))
    rawRef.current = formatBareNumber(next)
    dirty.current = false
    emit(next)
  }

  const onScrubPointerUp = (event: PointerEvent<HTMLElement>) => {
    const state = scrub.current
    scrub.current = null
    if (!state) return
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId)
    if (!state.dragging) {
      const input = event.currentTarget.parentElement?.querySelector('input')
      input?.focus()
      input?.select()
    }
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
      onWheel,
    },
    scrubProps: {
      onPointerDown: onScrubPointerDown,
      onPointerMove: onScrubPointerMove,
      onPointerUp: onScrubPointerUp,
      onPointerCancel: onScrubPointerUp,
      className: 'cursor-ew-resize select-none',
      style: { touchAction: 'none' as const },
    },
  }
}
