import { existsSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { lottieAnimations } from './vite-plugin-lottie'

const here = (path: string) => fileURLToPath(new URL(path, import.meta.url))
const dep = (name: string) => here(`./node_modules/${name}`)

function openPencilRoot(): string | null {
  if (process.env.OPEN_PENCIL_STUB === '1') return null
  const candidates = [
    process.env.OPEN_PENCIL_ROOT,
    here('../../open-pencil'),
    here('../../../open-pencil'),
  ].filter((item): item is string => !!item)
  return candidates.find((root) => existsSync(join(root, 'packages/fig/src/index.ts'))) ?? null
}

function openPencilAliases() {
  const root = openPencilRoot()
  if (!root) {
    return [
      { find: /^@open-pencil\/fig$/, replacement: here('./src/lib/design/openPencilStubs/fig.ts') },
      { find: /^@open-pencil\/pen$/, replacement: here('./src/lib/design/openPencilStubs/pen.ts') },
    ]
  }
  const src = (pkg: string) => join(root, 'packages', pkg, 'src')
  return [
    { find: /^@open-pencil\/scene-graph\/(.*)$/, replacement: `${src('scene-graph')}/$1` },
    { find: /^@open-pencil\/scene-graph$/, replacement: `${src('scene-graph')}/index.ts` },
    { find: /^@open-pencil\/kiwi\/(.*)$/, replacement: `${src('kiwi')}/$1` },
    { find: /^@open-pencil\/kiwi$/, replacement: `${src('kiwi')}/index.ts` },
    { find: /^@open-pencil\/fig\/(.*)$/, replacement: `${src('fig')}/$1` },
    { find: /^@open-pencil\/fig$/, replacement: `${src('fig')}/index.ts` },
    { find: /^@open-pencil\/pen\/(.*)$/, replacement: `${src('pen')}/$1` },
    { find: /^@open-pencil\/pen$/, replacement: `${src('pen')}/index.ts` },
    { find: 'fflate', replacement: dep('fflate') },
    { find: 'culori', replacement: dep('culori') },
    { find: 'es-toolkit', replacement: dep('es-toolkit') },
    { find: 'fzstd', replacement: dep('fzstd') },
    { find: 'js-base64', replacement: dep('js-base64') },
    { find: 'nanoevents', replacement: dep('nanoevents') },
    { find: 'svgpath', replacement: dep('svgpath') },
  ]
}

export default defineConfig({
  base: './',
  plugins: [react(), tailwindcss(), lottieAnimations()],
  resolve: {
    alias: openPencilAliases(),
  },
  build: { outDir: 'dist', emptyOutDir: true },
})
