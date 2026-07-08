import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { lottieAnimations } from './vite-plugin-lottie'

export default defineConfig({
  base: './',
  plugins: [react(), tailwindcss(), lottieAnimations()],
  build: { outDir: 'dist', emptyOutDir: true },
})
