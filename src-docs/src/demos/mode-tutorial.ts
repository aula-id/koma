import type { TutorialStep } from './first-run-tutorial'
import { commandEntryScreen, composeChatScreen } from './chat-chrome'

export function getModeSteps(rows = 24): TutorialStep[] {
  return [
    {
      title: 'Type /mode from Auto',
      narration: 'Start in Auto mode and type the bare /mode command. It is a chat command, not an overlay.',
      screen: commandEntryScreen(rows, '/mode', 'auto'),
    },
    {
      title: 'Normal chat',
      narration: 'Enter immediately returns to the ordinary chat screen with Normal active. Auto runs every requested tool; Normal still runs safe reads inline and prompts only before risky writes or deletes.',
      points: ['The changed header is the only mode-switch UI — no picker opens'],
      screen: composeChatScreen(rows, [], '', 80, 'normal'),
    },
    {
      title: 'Shift+Tab enters Plan',
      narration: 'Press real Shift+Tab (reported by Crossterm as BackTab) from Normal to advance to Plan. Plan is the read-only planning and exploration mode; its header reads “planning.”',
      screen: composeChatScreen(rows, [], '', 80, 'planning'),
    },
    {
      title: 'Cycle and explicit modes',
      narration: 'The ordinary bare /mode and Shift+Tab cycle is Auto → Normal → Plan → SDLC → Auto. /mode auto, /mode normal, /mode plan, and /mode sdlc set a named mode explicitly. Plan can also return after the model submits a plan for approval, restoring the mode active before Plan.',
      screen: composeChatScreen(rows, [], '', 80, 'sdlc'),
    },
    {
      title: 'SDLC and YOLO safeguards',
      narration: 'SDLC is phase-aware: bare /mode (or /mode exit) restores its prior mode, while named SDLC hops are allowed only during assess. Shift+Tab can leave SDLC only in assess or done; it remains locked during other phases such as execute and integrate. YOLO is not in the cycle: first arm “Enable YOLO mode” in /security, then explicitly use /mode yolo. It bypasses risky-tool approval but retains the workspace path guard.',
      screen: composeChatScreen(rows, [], '', 80, 'sdlc'),
    },
  ]
}
