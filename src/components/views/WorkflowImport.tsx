import { useRef } from 'react'
import { importWorkflowFile } from '@/lib/workflow-actions'

/**
 * A file picker for workflow YAML files. Render `input` anywhere and call
 * `pick` from a button. Works in the desktop webview and the browser.
 */
export function useWorkflowImport() {
  const inputRef = useRef<HTMLInputElement>(null)

  const input = (
    <input
      ref={inputRef}
      type="file"
      accept=".yaml,.yml"
      hidden
      data-testid="workflow-file-input"
      onChange={event => {
        const file = event.target.files?.[0]
        // Reset so picking the same file again still fires `change`
        event.target.value = ''
        if (file) void importWorkflowFile(file)
      }}
    />
  )

  return { pick: () => inputRef.current?.click(), input }
}
