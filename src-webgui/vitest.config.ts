import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { lottieAnimations } from './vite-plugin-lottie'
import { playwright } from '@vitest/browser-playwright'

export default defineConfig({
  server: { fs: { allow: [new URL('..', import.meta.url).pathname] } },
  plugins: [react(), tailwindcss(), lottieAnimations()],
  optimizeDeps: { include: ['react/jsx-dev-runtime', 'react-dom'] },
  test: {
    projects: [
      {
        extends: true,
        test: {
          name: 'logic',
          environment: 'jsdom',
          include: ['src/**/*.vitest.test.ts', 'src/**/*.vitest.test.tsx'],
        },
      },
      {
        extends: true,
        test: {
          name: 'chromium',
          include: ['src/**/*.browser.test.ts', 'src/**/*.browser.test.tsx'],
          browser: {
            enabled: true,
            headless: true,
            provider: playwright(),
            instances: [{ browser: 'chromium' }],
          },
        },
      },
    ],
  },
})
