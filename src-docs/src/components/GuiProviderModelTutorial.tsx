import { ArrowLeft, ArrowRight, Bot, Brain, Check, ChevronDown, KeyRound, Minus, Plus, Search, Server } from 'lucide-react'
import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import { GuiTutorialDesktopFrame, TutorialActivity, TutorialChat } from './gui-tutorial/TutorialDesktop'
import { GUI_TUTORIAL_CANVAS } from './gui-tutorial/model'

const ZOOM_LEVELS = [0.85, 1, 1.15, 1.3]
const CURSOR_FOCUS_ZOOM = 1.45
const CURSOR_TARGETS = [{ x: 24, y: 223 }, { x: 164, y: 105 }, { x: 165, y: 130 }, { x: 270, y: 290 }, { x: 168, y: 245 }, { x: 270, y: 290 }, { x: 1060, y: 610 }, { x: 760, y: 500 }]
const STEPS = [
  { title: 'Open Connector', instruction: 'Select Connector in the activity bar.' },
  { title: 'Add provider', instruction: 'In Providers, choose Add provider.' },
  { title: 'Choose provider', instruction: 'Choose OpenAI from the provider list.' },
  { title: 'Configure provider', instruction: 'Review the prefilled endpoint and enter your API key in the real GUI, then save.' },
  { title: 'Add model', instruction: 'In Models, choose Add model.' },
  { title: 'Configure model', instruction: 'Set a global main model using the provider and a model ID from its live catalogue.' },
  { title: 'Use model', instruction: 'Open the composer model picker and select the global model for this session.' },
  { title: 'Complete', instruction: 'Your selected model is now the active session model and you can start chatting.' },
]

/** Docs-local, deterministic Connector flow rendered inside the shared GUI tutorial shell. */
export function GuiProviderModelTutorial() {
  const [step, setStep] = useState(0)
  const [zoomIndex, setZoomIndex] = useState(1)
  const [click, setClick] = useState(0)
  const viewportRef = useRef<HTMLDivElement>(null)
  const [fitScale, setFitScale] = useState(1)
  useEffect(() => { const viewport = viewportRef.current; if (!viewport) return; const update = () => { const rect = viewport.getBoundingClientRect(); setFitScale(Math.min(rect.width / GUI_TUTORIAL_CANVAS.width, rect.height / GUI_TUTORIAL_CANVAS.height)) }; update(); const observer = new ResizeObserver(update); observer.observe(viewport); return () => observer.disconnect() }, [])
  const complete = step === STEPS.length - 1
  const move = (next: number) => { setStep(next); setClick(value => value + 1) }
  const zoom = ZOOM_LEVELS[zoomIndex]
  const cursor = CURSOR_TARGETS[step]
  const stageScale = fitScale * zoom * CURSOR_FOCUS_ZOOM
  const focusOffset = { x: -(cursor.x - GUI_TUTORIAL_CANVAS.width / 2) * stageScale, y: -(cursor.y - GUI_TUTORIAL_CANVAS.height / 2) * stageScale }
  return <section className="gpm-tutorial" aria-label="Desktop GUI provider and model tutorial">
    <header className="gpm-header"><div><span className="gpm-kicker">Scripted desktop emulation · no network or saved configuration</span><strong>Step {step + 1} of {STEPS.length}: {STEPS[step].title}</strong></div><div className="gpm-step-dots" aria-label={`Tutorial progress: step ${step + 1} of ${STEPS.length}`}>{STEPS.map((_, index) => <i key={index} className={index === step ? 'active' : index < step ? 'done' : ''} />)}</div><div className="gpm-zoom" aria-label="Stage zoom"><button type="button" onClick={() => setZoomIndex(n => Math.max(0, n - 1))} disabled={zoomIndex === 0} aria-label="Zoom out"><Minus size={13}/></button><button type="button" onClick={() => setZoomIndex(1)}>{Math.round(zoom * 100)}%</button><button type="button" onClick={() => setZoomIndex(n => Math.min(ZOOM_LEVELS.length - 1, n + 1))} disabled={zoomIndex === ZOOM_LEVELS.length - 1} aria-label="Zoom in"><Plus size={13}/></button></div></header>
    <div className="gpm-instruction"><span>{complete ? <Check size={16} /> : <Brain size={16} />}</span><p>{STEPS[step].instruction}</p></div>
    <div className="gui-tutorial-viewport gpm-viewport"><div ref={viewportRef} className="gui-tutorial-canvas"><div className="gui-tutorial-stage gpm-focus-stage" style={{ '--gui-tutorial-scale': stageScale, '--gpm-focus-x': `${focusOffset.x}px`, '--gpm-focus-y': `${focusOffset.y}px` } as CSSProperties}><GuiTutorialDesktopFrame><div className="gui-shell"><TutorialActivity active="Connector" onConnector={() => move(0)} /><aside className="gui-sidebar gpm-sidebar"><ConnectorView step={step} /></aside><div className="gui-sidebar-resize" /><main className="gui-main"><TutorialChat modelPicker={step === 6 ? <ModelPicker /> : undefined} /></main></div></GuiTutorialDesktopFrame><div aria-hidden="true" className="gui-tutorial-cursor pointer-events-none" style={{ left: cursor.x, top: cursor.y }}><svg viewBox="0 0 24 24"><path d="M4 2.5 19.2 12l-7.1 1.7L9 21.5 4 2.5Z"/><path d="m12 14 4.5 5"/></svg>{click > 0 && <span key={click} className="gui-tutorial-cursor-ring"/>}</div></div></div></div>
    <footer className="gpm-controls"><span>{complete ? 'Illustrative setup complete — nothing was sent, stored, or authenticated.' : 'Advance when you are ready; this tutorial never advances itself.'}</span><div><button type="button" onClick={() => move(Math.max(0, step - 1))} disabled={step === 0}><ArrowLeft size={14}/> Back</button>{complete ? <button type="button" onClick={() => move(0)}>Start over</button> : <button type="button" className="gpm-next" onClick={() => move(step + 1)}>Next <ArrowRight size={14}/></button>}</div></footer>
  </section>
}

function ConnectorView({ step }: { step: number }) {
  const providerSaved = step >= 4
  const modelSaved = step >= 6
  const providerForm = step === 2 || step === 3
  const modelForm = step === 5
  return <main className="gpm-connector"><div className="gpm-title">Connector</div>{providerForm ? <ProviderForm chosen={step === 3} /> : modelForm ? <ModelForm /> : <><Catalogue title="Providers" action="Add provider" target={step === 1}>{providerSaved ? <Row icon={<Server size={14}/>} title="OpenAI" subtitle="https://api.openai.com/v1" side="••••" /> : <Empty text="No providers" />}</Catalogue><Catalogue title="OAuth" action="Connect account"><Empty text="No connections" /></Catalogue><Catalogue title="Models" action="Add model" target={step === 4}>{modelSaved ? <Row icon={<Bot size={14}/>} title="GPT-4.1 main" subtitle="gpt-4.1 · OpenAI" side="main" /> : <Empty text="No models" />}</Catalogue>{step === 0 && <div className="gpm-callout">The Connector panel keeps Providers, OAuth connections, and Models in separate catalogues.</div>}</>}</main>
}

function ProviderForm({ chosen }: { chosen: boolean }) { return <div className="gpm-detail"><div className="gpm-detail-title">Add provider</div>{!chosen ? <><p className="gpm-section-label">Choose a provider</p><button className="gpm-provider-choice gpm-target"><Server size={15}/><span><b>OpenAI</b><small>https://api.openai.com/v1</small></span><ArrowRight size={14}/></button><button className="gpm-provider-choice"><Server size={15}/><span><b>OpenRouter</b><small>https://openrouter.ai/api/v1</small></span><ArrowRight size={14}/></button><button className="gpm-provider-choice"><span>⚙</span><span><b>Custom</b><small>Enter name + endpoint manually</small></span><ArrowRight size={14}/></button></> : <><Field label="Name" value="OpenAI"/><Field label="Endpoint (base URL)" value="https://api.openai.com/v1"/><Field label="API key" value="Illustrative value only — no real key" keyField/><p className="gpm-safe"><KeyRound size={13}/> Docs-only example. This emulator accepts no credentials and makes no request.</p><div className="gpm-form-actions"><button>Cancel</button><button className="gpm-target">Save</button></div></>}</div> }
function ModelForm() { return <div className="gpm-detail"><div className="gpm-detail-title">Add model</div><Field label="Name" value="GPT-4.1 main"/><Field label="Provider" value="OpenAI" select/><Field label="Model id" value="gpt-4.1 · illustrative catalogue value"/><div className="gpm-field"><label>Route</label><div className="gpm-radio"><Check size={13}/> Auto <small>Let the provider route the request.</small></div></div><div className="gpm-field"><label>Roles</label><div className="gpm-chip">main <Check size={11}/></div></div><Field label="Scope" value="Global" select/><div className="gpm-form-actions"><button>Cancel</button><button className="gpm-target">Save</button></div></div> }
function ModelPicker() { return <div className="gpm-picker"><button className="gpm-picker-trigger"><Bot size={14}/> (inherit) <ChevronDown size={13}/></button><div><Search size={12}/><span>Search models…</span><button><Check size={12}/> (inherit) — global main</button><button className="gpm-selected"><Check size={12}/> <span>GPT-4.1 main<small>gpt-4.1 · OpenAI</small></span></button></div></div> }
function Field({ label, value, select, keyField }: { label: string; value: string; select?: boolean; keyField?: boolean }) { return <div className="gpm-field"><label>{label}</label><div className={keyField ? 'gpm-input gpm-illustrative' : 'gpm-input'}>{value}{select && <ChevronDown size={13}/>}</div></div> }
function Catalogue({ title, action, target, children }: { title: string; action: string; target?: boolean; children: ReactNode }) { return <section className="gpm-catalogue"><header><b><ChevronDown size={13}/>{title}</b><button className={target ? 'gpm-target' : ''}><Plus size={13}/>{action}</button></header>{children}</section> }
function Row({ icon, title, subtitle, side }: { icon: ReactNode; title: string; subtitle: string; side: string }) { return <div className="gpm-row"><span>{icon}</span><div><b>{title}</b><small>{subtitle}</small></div><em>{side}</em></div> }
function Empty({ text }: { text: string }) { return <p className="gpm-empty">{text}</p> }
