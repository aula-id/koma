// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest'
import { cleanup, render, screen } from '@testing-library/react'
import { AccordionSection } from './AccordionSection'

afterEach(cleanup)

describe('AccordionSection', () => {
  it('exposes expanded state and controls only while open', () => {
    const { rerender } = render(<AccordionSection title="Files" open onToggle={() => {}}>body</AccordionSection>)
    const btn = screen.getByRole('button', { name: /Files/ })
    expect(btn.getAttribute('aria-expanded')).toBe('true')
    const id = btn.getAttribute('aria-controls')
    expect(id).toBeTruthy()
    expect(document.getElementById(id!)?.textContent).toBe('body')
    rerender(<AccordionSection title="Files" open={false} onToggle={() => {}}>body</AccordionSection>)
    const closed = screen.getByRole('button', { name: /Files/ })
    expect(closed.getAttribute('aria-expanded')).toBe('false')
    expect(closed.hasAttribute('aria-controls')).toBe(false)
  })
  it('renders count only when provided and the action slot', () => {
    const { rerender } = render(<AccordionSection title="A" open={false} onToggle={() => {}} action={<i data-testid="act" />} />)
    expect(screen.getByTestId('act')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'A' })).toBeTruthy()
    rerender(<AccordionSection title="A" open={false} onToggle={() => {}} count={3} />)
    expect(screen.getByRole('button', { name: /A\s*3/ })).toBeTruthy()
  })
})
