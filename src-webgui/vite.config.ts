import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { fileURLToPath } from 'node:url'
import { lottieAnimations } from './vite-plugin-lottie'

const here = (path: string) => fileURLToPath(new URL(path, import.meta.url))
const opSrc = (pkg: string) => here(`../../open-pencil/packages/${pkg}/src`)
const dep = (name: string) => here(`./node_modules/${name}`)

export default defineConfig({
  base: './',
  plugins: [react(), tailwindcss(), lottieAnimations()],
  resolve: {
    alias: [
      { find: /^@open-pencil\/scene-graph\/(.*)$/, replacement: `${opSrc('scene-graph')}/$1` },
      { find: /^@open-pencil\/scene-graph$/, replacement: `${opSrc('scene-graph')}/index.ts` },
      { find: /^@open-pencil\/kiwi\/(.*)$/, replacement: `${opSrc('kiwi')}/$1` },
      { find: /^@open-pencil\/kiwi$/, replacement: `${opSrc('kiwi')}/index.ts` },
      { find: /^@open-pencil\/fig\/(.*)$/, replacement: `${opSrc('fig')}/$1` },
      { find: /^@open-pencil\/fig$/, replacement: `${opSrc('fig')}/index.ts` },
      { find: /^@open-pencil\/pen\/(.*)$/, replacement: `${opSrc('pen')}/$1` },
      { find: /^@open-pencil\/pen$/, replacement: `${opSrc('pen')}/index.ts` },
      { find: 'fflate', replacement: dep('fflate') },
      { find: 'culori', replacement: dep('culori') },
      { find: 'es-toolkit', replacement: dep('es-toolkit') },
      { find: 'fzstd', replacement: dep('fzstd') },
      { find: 'js-base64', replacement: dep('js-base64') },
      { find: 'nanoevents', replacement: dep('nanoevents') },
      { find: 'svgpath', replacement: dep('svgpath') },
    ],
  },
  build: { outDir: 'dist', emptyOutDir: true },
})
