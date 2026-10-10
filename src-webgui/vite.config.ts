import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { lottieAnimations } from './vite-plugin-lottie'

export default defineConfig({
  server: { fs: { allow: [new URL('..', import.meta.url).pathname] } },
  base: './',
  plugins: [react(), tailwindcss(), lottieAnimations()],
  build: { outDir: 'dist', emptyOutDir: true },
})
