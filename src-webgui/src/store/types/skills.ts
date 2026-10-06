export type SkillScope = 'global' | 'project' | 'external'

export type SkillCatalogueEntry = {
  skillId: string
  generation: string
  name: string
  description: string
  triggers: string
  sourceTier: string
  scope: SkillScope
  sourcePath: string
  externalRootIndex?: number
}

export type SkillDetail = {
  skillId: string
  generation: string
  name: string
  description: string
  triggers: string
  declaredTools: string[]
  instruction: string
  companionFiles: string[]
  scope: SkillScope
  sourcePath: string
  externalRootIndex?: number
  editable: boolean
  structuredSaveSupported: boolean
}

export type SkillItemOutcome = {
  name: string
  status: 'success' | 'skipped' | 'failed' | 'partial'
  error?: string
}
