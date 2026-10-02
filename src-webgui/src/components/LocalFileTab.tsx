import { computerImageUrl } from './ComputerPreview'
import { EditorChrome } from './EditorChrome'
import { viewerKindForPath } from '../lib/viewerKind'
import type { Tab } from '../store/koma'

type LocalFileTab = Extract<Tab, { kind: 'localFile' }>

/** Read-only preview for session attachment files (absolute path on disk). */
export default function LocalFileTab({ tab }: { tab: LocalFileTab }) {
  const kind = viewerKindForPath(tab.absPath)
  return (
    <EditorChrome path={tab.absPath} status="Attachment preview">
      <div className="flex min-h-0 flex-1 items-center justify-center overflow-auto bg-koma-bg p-4">
        {kind === 'image' ? (
          <img
            src={computerImageUrl(tab.absPath)}
            alt={tab.title}
            className="max-h-full max-w-full object-contain"
          />
        ) : (
          <p className="text-[13px] text-koma-dim">
            Preview for this file type is not available here. Path: {tab.absPath}
          </p>
        )}
      </div>
    </EditorChrome>
  )
}
