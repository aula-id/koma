import { useEffect, useRef, useState, type CSSProperties } from 'react'
import { GuiTutorialCardHeader } from './gui-tutorial/GuiTutorialCardHeader'
import { TutorialDesktop } from './gui-tutorial/TutorialDesktop'
import { GUI_TUTORIAL_CANVAS, STAGE_LABELS, STAGE_TARGETS, type StageTarget, type TutorialStage } from './gui-tutorial/model'

const ZOOM_LEVELS = [0.85, 1, 1.15, 1.3]
const NARRATION: Record<TutorialStage, string> = {
  loading: 'The real onboarding gate keeps session chrome unavailable while configuration loads; this deterministic tutorial mirrors that gate.',
  theme: 'First-run setup is shown beneath the titlebar. This is an illustrative walkthrough of the supported setup state.',
  connect: 'Koma Free is the scripted keyless setup choice. Other connection choices remain separate options in the real application.',
  settingUp: 'The illustrative setup waits for provider and model configuration to be saved before leaving onboarding.',
  start: 'Configuration is complete, so the production no-session start screen appears. Its Recent card truthfully has no sessions; no workspace, project activity, or chat is attached.',
  session: 'New session follows the GUI’s attached-session shell with an empty transcript, empty Explorer activity, composer, and usage footer. The deterministic documentation fixture is koma (portable label ~/projects/koma); the current production Explore panel does not render a filesystem tree.',
}

export function GuiFirstRunTutorial() {
  const [stage, setStage] = useState<TutorialStage>('loading')
  const [target, setTarget] = useState<StageTarget>(STAGE_TARGETS.loading)
  const [click, setClick] = useState(0)
  const [playing, setPlaying] = useState(true)
  const [zoomIndex, setZoomIndex] = useState(1)
  const [reducedMotion, setReducedMotion] = useState(false)
  const viewportRef = useRef<HTMLDivElement>(null)
  const [fitScale, setFitScale] = useState(1)
  useEffect(() => { const viewport = viewportRef.current; if (!viewport) return; const update = () => { const r = viewport.getBoundingClientRect(); setFitScale(Math.min(r.width / GUI_TUTORIAL_CANVAS.width, r.height / GUI_TUTORIAL_CANVAS.height)) }; update(); const observer = new ResizeObserver(update); observer.observe(viewport); return () => observer.disconnect() }, [])
  useEffect(() => { const query = window.matchMedia('(prefers-reduced-motion: reduce)'); const update = () => setReducedMotion(query.matches); update(); query.addEventListener('change', update); return () => query.removeEventListener('change', update) }, [])
  const move = (next: TutorialStage, hotspot = STAGE_TARGETS[next], showClick = false) => { setTarget(hotspot); if (showClick) setClick(value => value + 1); setStage(next) }
  useEffect(() => { if (!playing || reducedMotion) return; const timers = [window.setTimeout(() => move('theme'), 750), window.setTimeout(() => move('theme', STAGE_TARGETS.theme, true), 1450), window.setTimeout(() => move('connect', { x: 820, y: 599 }, true), 2150), window.setTimeout(() => move('settingUp', STAGE_TARGETS.connect, true), 3050), window.setTimeout(() => move('start'), 4200), window.setTimeout(() => setPlaying(false), 4300)]; return () => timers.forEach(window.clearTimeout) }, [playing, reducedMotion])
  const replay = () => { setPlaying(false); move('loading'); window.setTimeout(() => setPlaying(true), 0) }
  const stopMove = (next: TutorialStage, hotspot?: StageTarget) => { setPlaying(false); move(next, hotspot, true) }
  const setupFree = () => { stopMove('settingUp'); window.setTimeout(() => move('start'), reducedMotion ? 0 : 900) }
  const zoom = ZOOM_LEVELS[zoomIndex]
  return <section className="overflow-hidden rounded-lg border border-koma-border bg-koma-panel" aria-label="Desktop GUI first-run tutorial">
    <GuiTutorialCardHeader eyebrow="Scripted desktop emulation" title={STAGE_LABELS[stage]} zoom={zoom} zoomIndex={zoomIndex} zoomLevels={ZOOM_LEVELS} onZoomIndex={setZoomIndex} action={{ label: 'Replay', onClick: replay }}/>
    <div className="border-b border-koma-border bg-koma-panel2 px-5 py-4 text-sm leading-relaxed text-koma-fg">{NARRATION[stage]}</div>
    <div className="gui-tutorial-viewport bg-koma-bg"><div ref={viewportRef} className="gui-tutorial-canvas"><div className="gui-tutorial-stage" style={{ '--gui-tutorial-scale': fitScale * zoom } as CSSProperties}><TutorialDesktop stage={stage} onTheme={() => stopMove('theme')} onNext={() => stopMove('connect', { x: 820, y: 599 })} onFree={setupFree} onSession={() => stopMove('session')}/><div aria-hidden="true" className="gui-tutorial-cursor pointer-events-none" style={{ left: target.x, top: target.y }}><svg viewBox="0 0 24 24"><path d="M4 2.5 19.2 12l-7.1 1.7L9 21.5 4 2.5Z"/><path d="m12 14 4.5 5"/></svg>{click > 0 && <span key={click} className="gui-tutorial-cursor-ring"/>}</div></div></div></div>
    <div className="flex items-center justify-between bg-koma-panel px-4 py-2 text-[11px] text-koma-dim"><span>{reducedMotion ? 'Reduced motion: autoplay is paused and zoom changes instantly.' : 'Use zoom controls to inspect the complete desktop window.'}</span><span>Click the illustrative controls to explore.</span></div>
  </section>
}
