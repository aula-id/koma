/// <reference types="vite/client" />

// Build-time-generated Lottie animations (see vite-plugin-lottie.ts): the inner
// animation JSON of each public/lottie/*.lottie archive, images inlined. Rendered
// by lottie-react in the waiting indicator.
declare module 'virtual:lottie-animations' {
  const animations: object[]
  export default animations
}

// Monaco deep ESM paths used for peek-references force path. Package exports
// map `"./*": "./*"` without a types condition, so tsc needs these shims.
declare module 'monaco-editor/esm/vs/editor/standalone/browser/standaloneServices.js' {
  export const StandaloneServices: {
    get: <T = unknown>(id: unknown) => T
    initialize: (overrides?: unknown) => unknown
  }
}

declare module 'monaco-editor/esm/vs/platform/commands/common/commands.js' {
  export const ICommandService: unknown
  export const CommandsRegistry: unknown
}
