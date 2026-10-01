export type FramePreset = { name: string; width: number; height: number }

export const FRAME_PRESETS: FramePreset[] = [
  { name: 'Phone', width: 390, height: 844 },
  { name: 'Tablet', width: 768, height: 1024 },
  { name: 'Desktop', width: 1440, height: 900 },
  { name: 'Slide', width: 1920, height: 1080 },
  { name: 'A4', width: 794, height: 1123 },
]

export function applyFramePreset<T extends { w: number; h: number; wMode?: string; hMode?: string }>(node: T, preset: FramePreset): T {
  return { ...node, w: preset.width, h: preset.height, wMode: 'fixed', hMode: 'fixed' }
}
