/**
 * Keyboard shortcuts are shown in the real searchable `/help` screen, not in a
 * separate TUI page. Kept as a compatibility export for any external consumers.
 */

import { getHelpSteps } from './help-tutorial'
import type { TutorialStep } from './first-run-tutorial'

/** Return the honest `/help` tutorial, which includes the keyboard reference. */
export function getKeyboardShortcutsSteps(rows = 24): TutorialStep[] {
  return getHelpSteps(rows)
}
