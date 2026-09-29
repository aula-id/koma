// Browser-only fixture; requires a running GUI dev server and headless Chrome with CDP.
// Never requests desktop capture or input. See docs/computer-use.md.
import assert from 'node:assert/strict'
const cdp = process.env.KOMA_BROWSER_CDP_URL ?? 'http://127.0.0.1:9222'
const gui = process.env.KOMA_GUI_URL ?? 'http://127.0.0.1:5173'
const pages = await (await fetch(new URL('/json', cdp))).json()
const socket = new WebSocket(pages.find(p => p.type === 'page').webSocketDebuggerUrl)
await new Promise(resolve => socket.addEventListener('open', resolve, { once: true }))
let id = 0
const pending = new Map()
socket.addEventListener('message', event => {
  const data = JSON.parse(event.data)
  if (pending.has(data.id)) {
    const { resolve, reject } = pending.get(data.id)
    pending.delete(data.id)
    data.error ? reject(Error(JSON.stringify(data.error))) : resolve(data.result)
  }
})
function send(method, params = {}) {
  return new Promise((resolve, reject) => { pending.set(++id, { resolve, reject }); socket.send(JSON.stringify({ id, method, params })) })
}
async function evaluate(expression) {
  const result = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true })
  if (result.exceptionDetails) throw Error(JSON.stringify(result.exceptionDetails))
  return result.result.value
}
async function waitFor(expression) {
  for (let i = 0; i < 120; i++) {
    if (await evaluate(`Boolean(${expression})`)) return
    await new Promise(r => setTimeout(r, 100))
  }
  throw Error('Timed out: ' + expression)
}
const source = (id, title, application) => ({ id, title, application, geometry: { x: 0, y: 0, width: 1920, height: 1080 }, focused: false })
const status = { session: 's', generation: 'g', enabled: true, paused: false, busy: false, observation: null, message: '', desktop: 'fixture', capabilities: { capture: true, windows: true, pointer: true, keyboard: true }, windows: [source('display:1', 'Main display', 'Desktop'), ...Array.from({length: 10}, (_, i) => source(`${100+i}:456`, `Window ${i+1}`, 'Fixture app'))] }
try {
  await send('Runtime.enable')
  await send('Page.enable')
  await send('Page.addScriptToEvaluateOnNewDocument', { source: `window.__komaComputerInitial=${JSON.stringify(status)};window.requests=[];window.ipc={postMessage:m=>window.requests.push(JSON.parse(m))};` })
  const navigation=String(Date.now())
  await send('Page.navigate', { url: new URL(`/?picker_test=${navigation}#computer-preview`, gui).href })
  await waitFor(`location.search.includes('${navigation}') && document.querySelector("button[aria-controls=computer-display-picker]")`)
  await evaluate('document.querySelector("button[aria-controls=computer-display-picker]").click()')
  await waitFor('document.querySelector("#computer-display-picker")')
  const visible = selector => evaluate(`(() => {
    const el=document.querySelector(${JSON.stringify(selector)});if(!el)return false;
    const r=el.getBoundingClientRect();
    return r.width>0 && r.height>0 && r.left>=0 && r.right<=innerWidth && r.top>=0 && r.bottom<=innerHeight && el.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2));
  })()`)
  const apps='button[aria-controls="computer-sources-application"]'
  const screens='button[aria-controls="computer-sources-screen"]'
  async function click(selector) {
    const rect=await evaluate(`(() => {const r=document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2}})()`)
    await send('Input.dispatchMouseEvent',{type:'mousePressed',button:'left',clickCount:1,...rect})
    await send('Input.dispatchMouseEvent',{type:'mouseReleased',button:'left',clickCount:1,...rect})
    await new Promise(r=>setTimeout(r,50))
  }
  for (const [width,height] of [[560,315],[420,236],[240,135],[320,120]]) {
    await send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false })
    await new Promise(r => setTimeout(r, 100))
    assert(await visible(screens),`Screens category visible at ${width}x${height}`)
    assert(await visible(apps),`Applications category visible at ${width}x${height}`)
    await click(apps)
    await evaluate('document.querySelector("#computer-sources-application").scrollTop=0')
    assert(await visible('#computer-sources-application button'),`First app row visible at ${width}x${height}`)
    assert(await evaluate('document.querySelector("#computer-sources-screen").hidden'))
    await evaluate('document.querySelector("#computer-sources-application").scrollTop=10000')
    assert(await visible('#computer-sources-application button:last-child'),`Last app row reachable at ${width}x${height}`)
    console.log(`PASS category visibility and scrollable app rows at ${width}x${height}`)
    await click(screens)
    await evaluate('document.querySelector("#computer-sources-application").scrollTop=0')
  }
  await click(apps)
  await evaluate('document.querySelector("#computer-sources-application").scrollTop=0')
  await click('#computer-sources-application button')
  assert((await evaluate('window.requests')).some(r=>r.action==='select' && r.window==='100:456'))
  assert(await evaluate('!document.querySelector("#computer-display-picker")'))
  assert(await evaluate('document.activeElement===document.querySelector("button[aria-controls=computer-display-picker]")'))
  console.log('PASS application selection routes window identity, closes picker and restores focus')
  await evaluate(`window.__komaComputerInitial=${JSON.stringify({...status,busy:true})};window.dispatchEvent(new CustomEvent('koma-computer',{detail:window.__komaComputerInitial}));window.requests=[];`)
  await new Promise(r=>setTimeout(r,50))
  await click('button[aria-controls=computer-display-picker]')
  assert(!(await evaluate('window.requests')).some(r=>r.action==='windows'))
  await evaluate(`window.dispatchEvent(new CustomEvent('koma-computer',{detail:{...window.__komaComputerInitial,busy:false}}))`)
  await waitFor('window.requests.some(r=>r.action==="windows")')
  assert.equal((await evaluate('window.requests')).filter(r=>r.action==='windows').length,1)
  await evaluate(`window.dispatchEvent(new CustomEvent('koma-computer',{detail:{...window.__komaComputerInitial,busy:false}}))`)
  await new Promise(r=>setTimeout(r,50))
  assert.equal((await evaluate('window.requests')).filter(r=>r.action==='windows').length,1)
  console.log('PASS opening while busy refreshes once when idle')
  await click('button[aria-label="Close source picker"]')
  await evaluate(`window.dispatchEvent(new CustomEvent('koma-computer',{detail:${JSON.stringify({...status,windows:[],capabilities:{...status.capabilities,windows:false}})}}))`)
  await new Promise(r=>setTimeout(r,50))
  await click('button[aria-controls=computer-display-picker]')
  await click(apps)
  assert(await visible('#computer-sources-application button'))
  await click('#computer-sources-application button')
  assert((await evaluate('window.requests')).some(r=>r.action==='select' && r.window==='portal:choose:application'))
  console.log('PASS Wayland Applications category opens application portal picker')
} finally { socket.close() }
