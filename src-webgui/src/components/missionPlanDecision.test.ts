import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

// Lightweight source-level regression (no React test runner in package.json):
// mission_ready / plan_ready must emit PlanDecision, never ApproveTool.
// Server-side enforce lives in Rust (`chat.rs::approval_tests`).

const here = dirname(fileURLToPath(import.meta.url))
const chatView = readFileSync(join(here, 'ChatView.tsx'), 'utf8')
const approvalOverlay = readFileSync(join(here, 'ApprovalOverlay.tsx'), 'utf8')

{
  assert.match(
    chatView,
    /call\.name === 'plan_ready' \|\| call\.name === 'mission_ready'/,
    'ChatView must special-case plan_ready and mission_ready',
  )
  assert.match(
    chatView,
    /req\(\{\s*r:\s*'PlanDecision',\s*decision\s*\}\)/,
    'mission/plan ready controls must emit PlanDecision',
  )
  assert.doesNotMatch(
    chatView,
    /r:\s*'ApproveTool'/,
    'ChatView must not emit ApproveTool for plan/mission ready',
  )
}

{
  assert.match(
    approvalOverlay,
    /pending\.name === 'plan_ready' \|\| pending\.name === 'mission_ready'/,
    'ApprovalOverlay must skip plan_ready and mission_ready',
  )
  // ApproveTool only for non-ready risky tools.
  assert.match(
    approvalOverlay,
    /r:\s*'ApproveTool'/,
    'ApprovalOverlay still uses ApproveTool for ordinary risky tools',
  )
}

console.log('missionPlanDecision.test.ts: ok')
