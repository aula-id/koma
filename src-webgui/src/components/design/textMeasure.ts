import { setDesignTextMeasurer, type DesignTextFont } from '../../lib/design'
import { weightCss } from './tabShared'

/**
 * Real text widths for auto-sized text, from the browser's own font metrics. Text with no
 * family of its own renders in the app's UI font, so that is what it is measured in.
 */
export function installDesignTextMeasurer(): void {
  if (typeof document === 'undefined') return
  const context = document.createElement('canvas').getContext('2d')
  if (!context) return
  let uiFamily: string | null = null
  setDesignTextMeasurer((text: string, font: DesignTextFont) => {
    uiFamily ??= getComputedStyle(document.body).fontFamily || 'sans-serif'
    context.font = `${font.italic ? 'italic ' : ''}${weightCss(font.weight)} ${font.size}px ${font.family || uiFamily}`
    return context.measureText(text).width
  })
}
