import { getTaskSteps } from './src/demos/task-tutorial'
import { getBashSteps } from './src/demos/bash-tutorial'
import { getUsageSteps } from './src/demos/usage-tutorial'

function stripAnsi(s: string): string {
  return s.replace(/\x1b\[[0-9;]*m/g, '')
}

function verifyScreen(name: string, screen: string) {
  const lines = screen.split('\n')
  const lineCount = lines.length
  let maxVisLen = 0
  let minVisLen = 999
  let badLines: number[] = []
  for (let i = 0; i < lines.length; i++) {
    const visLen = stripAnsi(lines[i]).length
    if (visLen > 120) {
      badLines.push(i)
    }
    maxVisLen = Math.max(maxVisLen, visLen)
    minVisLen = Math.min(minVisLen, visLen)
  }
  const ok = lineCount === 48 && badLines.length === 0
  console.log(`${ok ? '✅' : '❌'} ${name}: ${lineCount} lines, vis len ${minVisLen}-${maxVisLen}` +
    (badLines.length > 0 ? ` [OVER: lines ${badLines.join(',')}]` : ''))
  if (!ok && lineCount !== 48) {
    console.log(`   Expected 48 lines, got ${lineCount}`)
  }
}

const taskSteps = getTaskSteps(48)
for (const step of taskSteps) {
  verifyScreen(`task: ${step.title}`, step.screen)
}

const bashSteps = getBashSteps(48)
for (const step of bashSteps) {
  verifyScreen(`bash: ${step.title}`, step.screen)
}

const usageSteps = getUsageSteps(48)
for (const step of usageSteps) {
  verifyScreen(`usage: ${step.title}`, step.screen)
}
