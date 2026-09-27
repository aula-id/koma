export type ComputerRect = { x: number; y: number; width: number; height: number }
export type ComputerWindow = { id: string; application: string; title: string; geometry: ComputerRect; focused: boolean }
export type ComputerObservation = {
  id: string; session: string; generation: string; window: ComputerWindow
  transform: { desktop: ComputerRect; width: number; height: number }
  captured_ms: number; image_path: string
  accessibility_status: string; ocr_status: string
  elements: { id: string; source: string; label: string; role: string; bounds: ComputerRect; enabled: boolean; selected: boolean; focused: boolean; confidence: number | null }[]
}
export type ComputerStatus = {
  session: string; desktop: string; generation: string; enabled: boolean; paused: boolean; busy: boolean
  capabilities: { capture: boolean; windows: boolean; focus: boolean; pointer: boolean; keyboard: boolean; accessibility: boolean; ocr: boolean; floating: boolean; limitations: string[] }
  observation: ComputerObservation | null; windows: ComputerWindow[]; message: string
}
