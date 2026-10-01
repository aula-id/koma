export function parsePenFile(_json: string): { getPages?: () => unknown[]; getNode?: (id: string) => unknown } {
  throw new Error('OpenPencil pen parser is not available in this build')
}
