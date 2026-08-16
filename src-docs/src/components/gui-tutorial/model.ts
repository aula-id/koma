export const GUI_TUTORIAL_CANVAS = { width: 1280, height: 720 } as const

export type TutorialStage = 'loading' | 'theme' | 'connect' | 'settingUp' | 'start' | 'session'
export type StageTarget = { x: number; y: number }

/** Deterministic documentation fixture derived from this repository's checked-in tree. */
export const KOMA_GUI_TUTORIAL_FIXTURE = {
  name: 'koma',
  rootLabel: '~/projects/koma',
  tree: [
    'Cargo.toml', 'README.md', 'src-agent/', 'src-webgui/', 'src-docs/',
  ],
} as const

// Coordinates are viewport pixels in the canonical, unscaled 1280×720 stage.
export const STAGE_TARGETS: Record<TutorialStage, StageTarget> = {
  loading: { x: 640, y: 365 },
  theme: { x: 520, y: 260 },
  connect: { x: 640, y: 285 },
  settingUp: { x: 640, y: 285 },
  start: { x: 650, y: 330 },
  session: { x: 790, y: 620 },
}

export const STAGE_LABELS: Record<TutorialStage, string> = {
  loading: 'Starting', theme: 'Theme', connect: 'Connect', settingUp: 'Setting up', start: 'No session', session: 'New session',
}
