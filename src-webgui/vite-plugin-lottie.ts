import { readdirSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { unzipSync } from 'fflate'
import type { Plugin } from 'vite'

// Build-time dotLottie extractor.
//
// The files in public/lottie/*.lottie are dotLottie archives (ZIP: manifest.json
// + animations/*.json + optional images/*). The koma:// custom protocol serves
// the built dist tree with a no-wasm rule (see MessageBody.tsx / komaShiki.ts),
// so we deliberately AVOID @lottiefiles/dotlottie-react (which ships a .wasm).
// Instead we unzip each archive at build, pull out the inner Lottie animation
// JSON, inline any externally-referenced raster assets as base64 data URIs, and
// expose the parsed animations as a `virtual:lottie-animations` module that a
// pure-JS player (lottie-react) can render directly. Zero protocol changes.

const VIRTUAL_ID = 'virtual:lottie-animations'
const RESOLVED_ID = '\0' + VIRTUAL_ID

const MIME: Record<string, string> = {
  webp: 'image/webp',
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  gif: 'image/gif',
  svg: 'image/svg+xml',
  bmp: 'image/bmp',
}

type LottieAsset = {
  p?: string
  u?: string
  e?: number
  [key: string]: unknown
}

function extractAnimations(dir: string): unknown[] {
  let files: string[]
  try {
    files = readdirSync(dir).filter((f) => f.toLowerCase().endsWith('.lottie'))
  } catch {
    // No lottie folder — the waiting indicator falls back to its pulse.
    return []
  }

  const out: unknown[] = []
  for (const file of files) {
    try {
      const buf = readFileSync(resolve(dir, file))
      const entries = unzipSync(new Uint8Array(buf))

      // The inner Lottie animation lives at animations/<name>.json.
      const animPath = Object.keys(entries).find((p) => /^animations\/.+\.json$/i.test(p))
      if (!animPath) continue

      const anim = JSON.parse(Buffer.from(entries[animPath]).toString('utf8')) as {
        assets?: LottieAsset[]
      }

      // Inline externally-referenced images (e !== 1) as data URIs so lottie-web
      // renders them without needing the surrounding zip. dotLottie references
      // them as { u: "/images/", p: "1.webp", e: 0 }.
      for (const asset of anim.assets ?? []) {
        if (!asset || asset.e === 1 || typeof asset.p !== 'string' || asset.p === '') continue
        const u = typeof asset.u === 'string' ? asset.u.replace(/^\/+|\/+$/g, '') : ''
        const candidates = [u ? `${u}/${asset.p}` : asset.p, `images/${asset.p}`, asset.p]
        const zipPath = candidates.find((c) => entries[c])
        if (!zipPath) continue
        const ext = asset.p.split('.').pop()?.toLowerCase() ?? ''
        const mime = MIME[ext] ?? 'application/octet-stream'
        const b64 = Buffer.from(entries[zipPath]).toString('base64')
        asset.p = `data:${mime};base64,${b64}`
        asset.u = ''
        asset.e = 1
      }

      out.push(anim)
    } catch {
      // Skip an unreadable / malformed archive; the rest still load.
    }
  }
  return out
}

export function lottieAnimations(): Plugin {
  let dir = ''
  return {
    name: 'koma-lottie-animations',
    configResolved(config) {
      dir = resolve(config.root, 'public/lottie')
    },
    resolveId(id) {
      return id === VIRTUAL_ID ? RESOLVED_ID : undefined
    },
    load(id) {
      if (id !== RESOLVED_ID) return undefined
      const anims = extractAnimations(dir)
      return `export default ${JSON.stringify(anims)}`
    },
  }
}
