import { ArrowLeft, ArrowRight } from 'lucide-react'
import { useEffect, useRef, useState, type CSSProperties } from 'react'
import { EmulatorJoyride } from './gui-tutorial/EmulatorJoyride'
import { GuiTutorialCardHeader } from './gui-tutorial/GuiTutorialCardHeader'
import { TutorialDesktop } from './gui-tutorial/TutorialDesktop'
import { GUI_TUTORIAL_CANVAS, STAGE_LABELS, type TutorialStage } from './gui-tutorial/model'

const ZOOM_LEVELS = [0.85, 1, 1.15, 1.3]
const STAGES: { stage: TutorialStage; target: string; narration: string }[] = [
  { stage: 'loading', target: 'desktop', narration: 'The onboarding gate keeps session chrome unavailable while configuration loads.' },
  { stage: 'theme', target: 'theme-dark', narration: 'Pick a theme. The selected option repaints the application immediately.' },
  { stage: 'connect', target: 'koma-free', narration: 'Choose how to connect a model. This guided path demonstrates the keyless Koma Free choice.' },
  { stage: 'settingUp', target: 'koma-free', narration: 'The illustrative setup is preparing its deterministic provider and model configuration.' },
  { stage: 'start', target: 'new-session', narration: 'Configuration is complete. The no-session screen truthfully has no workspace, project activity, or chat attached.' },
  { stage: 'session', target: 'composer', narration: 'The attached-session shell starts with an empty transcript, Explorer activity, composer, and usage footer.' },
]

/** Manual, deterministic first-run walkthrough backed by the docs-local desktop fixture. */
export function GuiFirstRunTutorial() {
  const [stageIndex, setStageIndex] = useState(0)
  const [zoomIndex, setZoomIndex] = useState(1)
  const viewportRef = useRef<HTMLDivElement>(null)
  const [fitScale, setFitScale] = useState(1)
  const current = STAGES[stageIndex]
  const zoom = ZOOM_LEVELS[zoomIndex]

  useEffect(() => {
    const viewport = viewportRef.current
    if (!viewport) return
    const update = () => {
      const rect = viewport.getBoundingClientRect()
      setFitScale(Math.min(rect.width / GUI_TUTORIAL_CANVAS.width, rect.height / GUI_TUTORIAL_CANVAS.height))
    }
    update()
    const observer = new ResizeObserver(update)
    observer.observe(viewport)
    return () => observer.disconnect()
  }, [])

  const moveTo = (stage: TutorialStage) => setStageIndex(STAGES.findIndex((item) => item.stage === stage))
  const complete = stageIndex === STAGES.length - 1

  return <section className="overflow-hidden rounded-lg border border-koma-border bg-koma-panel" aria-label="Desktop GUI first-run tutorial">
    <GuiTutorialCardHeader eyebrow="Scripted desktop emulation" title={`Step ${stageIndex + 1} of ${STAGES.length}: ${STAGE_LABELS[current.stage]}`} zoom={zoom} zoomIndex={zoomIndex} zoomLevels={ZOOM_LEVELS} onZoomIndex={setZoomIndex}/>
    <div className="border-b border-koma-border bg-koma-panel2 px-5 py-4 text-sm leading-relaxed text-koma-fg">{current.narration}</div>
    <div className="gui-tutorial-viewport bg-koma-bg"><div ref={viewportRef} className="gui-tutorial-canvas"><div className="gui-tutorial-stage" style={{ '--gui-tutorial-scale': fitScale * zoom } as CSSProperties}><TutorialDesktop stage={current.stage} onTheme={() => moveTo('theme')} onNext={() => moveTo('connect')} onFree={() => moveTo('settingUp')} onSession={() => moveTo('session')}/></div><div id="first-run-tour-portal" className="absolute inset-0 overflow-hidden"/><EmulatorJoyride portalId="first-run-tour-portal" target={current.target} title={STAGE_LABELS[current.stage]} description={current.narration}/></div></div>
    <footer className="flex items-center justify-between gap-3 bg-koma-panel px-4 py-2 text-[11px] text-koma-dim"><span>{complete ? 'Illustrative first-run complete — no configuration was saved.' : 'Advance when ready; this tutorial never advances itself.'}</span><div className="flex gap-2"><button type="button" onClick={() => setStageIndex((value) => Math.max(0, value - 1))} disabled={stageIndex === 0} className="flex items-center gap-1 rounded border border-koma-border px-2 py-1 disabled:opacity-35"><ArrowLeft size={13}/> Back</button>{complete ? <button type="button" onClick={() => setStageIndex(0)} className="rounded border border-koma-border px-2 py-1">Start over</button> : <button type="button" onClick={() => setStageIndex((value) => value + 1)} className="flex items-center gap-1 rounded border border-koma-border px-2 py-1 text-koma-accent">Next <ArrowRight size={13}/></button>}</div></footer>
  </section>
}
