import { ArrowLeft, ArrowRight, Brain, Check } from 'lucide-react'
import { useEffect, useRef, useState, type CSSProperties } from 'react'
import { ConnectorPanel, ModelPicker, useConnectorTutorial } from './gui-provider-model/ConnectorTutorial'
import { GuiTutorialCardHeader } from './gui-tutorial/GuiTutorialCardHeader'
import { EmulatorJoyride } from './gui-tutorial/EmulatorJoyride'
import { GuiTutorialDesktopFrame, TutorialActivity, TutorialChat } from './gui-tutorial/TutorialDesktop'
import { GUI_TUTORIAL_CANVAS } from './gui-tutorial/model'

const ZOOM_LEVELS = [0.85, 1, 1.15, 1.3]
const STEPS = [
  { title: 'Open Connector', instruction: 'Select Connector in the activity bar.', target: 'connector' },
  { title: 'Add provider', instruction: 'In Providers, choose Add provider.', target: 'add-provider' },
  { title: 'Choose provider', instruction: 'Choose OpenAI from the provider list.', target: 'openai' },
  { title: 'Configure provider', instruction: 'Review the prefilled endpoint and enter an API key in the real GUI, then save.', target: 'save-provider' },
  { title: 'Add model', instruction: 'In Models, choose Add model.', target: 'add-model' },
  { title: 'Configure model', instruction: 'Set a global main model using the provider and a model ID from its live catalogue.', target: 'save-model' },
  { title: 'Use model', instruction: 'Open the composer model picker and select the global model for this session.', target: 'model-picker' },
  { title: 'Complete', instruction: 'Your selected model is now the active session model and you can start chatting.', target: undefined },
] as const

/** Deterministic, interactive rendering of the WebGUI Connector flow using sample data only. */
export function GuiProviderModelTutorial() {
  const [step, setStep] = useState(0)
  const [zoomIndex, setZoomIndex] = useState(1)
  const viewportRef = useRef<HTMLDivElement>(null)
  const [fitScale, setFitScale] = useState(1)
  const controller = useConnectorTutorial(step)

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

  const complete = step === STEPS.length - 1
  const move = (next: number) => setStep(next)
  const zoom = ZOOM_LEVELS[zoomIndex]
  const current = STEPS[step]

  return <section className="gpm-tutorial" aria-label="Desktop GUI provider and model tutorial">
    <GuiTutorialCardHeader
      eyebrow="Interactive WebGUI Connector rendering · sample data only · no network or saved configuration"
      title={`Step ${step + 1} of ${STEPS.length}: ${current.title}`}
      zoom={zoom}
      zoomIndex={zoomIndex}
      zoomLevels={ZOOM_LEVELS}
      onZoomIndex={setZoomIndex}
    >
      <div className="gpm-step-dots" aria-label={`Tutorial progress: step ${step + 1} of ${STEPS.length}`}>{STEPS.map((_, index) => <i key={index} className={index === step ? 'active' : index < step ? 'done' : ''}/>)}</div>
    </GuiTutorialCardHeader>
    <div className="gpm-instruction"><span>{complete ? <Check size={16}/> : <Brain size={16}/>}</span><p>{current.instruction}</p></div>
    <div className="gui-tutorial-viewport gpm-viewport"><div ref={viewportRef} className="gui-tutorial-canvas"><div className="gui-tutorial-stage" style={{ '--gui-tutorial-scale': fitScale * zoom } as CSSProperties}>
      <GuiTutorialDesktopFrame><div className="gui-shell"><TutorialActivity active="Connector" onConnector={() => move(0)} highlightConnector={current.target === 'connector'}/><aside className="gui-sidebar gpm-sidebar"><header>Connector</header><div className="relative min-h-0 flex-1"><ConnectorPanel controller={controller} target={current.target}/></div></aside><div className="gui-sidebar-resize"/><main className="gui-main"><TutorialChat modelPicker={step >= 6 ? <ModelPicker models={controller.models} sessionMain={controller.sessionMain} onSelect={controller.setSessionMain} target={current.target === 'model-picker'}/> : undefined}/></main></div></GuiTutorialDesktopFrame>
    </div><div id="provider-model-tour-portal" className="absolute inset-0 overflow-hidden"/><EmulatorJoyride portalId="provider-model-tour-portal" target={current.target} title={current.title} description={current.instruction}/></div></div>
    <footer className="gpm-controls"><span>{complete ? 'Illustrative setup complete — no key, provider, model, or authentication was created.' : 'Advance when ready; direct controls only change local sample state.'}</span><div><button type="button" onClick={() => move(Math.max(0, step - 1))} disabled={step === 0}><ArrowLeft size={14}/> Back</button>{complete ? <button type="button" onClick={() => move(0)}>Start over</button> : <button type="button" className="gpm-next" onClick={() => move(step + 1)}>Next <ArrowRight size={14}/></button>}</div></footer>
  </section>
}
