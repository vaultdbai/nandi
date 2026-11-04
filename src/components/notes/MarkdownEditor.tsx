import { useEffect, useRef } from 'react'
import { markdown } from '@codemirror/lang-markdown'
import { EditorState } from '@codemirror/state'
import { EditorView, keymap, placeholder as hint } from '@codemirror/view'
import { minimalSetup } from 'codemirror'

/** Colors from the app's theme tokens, so light and dark both work */
const theme = EditorView.theme({
  '&': {
    height: '100%',
    fontSize: '0.875rem',
    color: 'var(--foreground)',
    backgroundColor: 'var(--background)',
  },
  '.cm-scroller': {
    fontFamily: 'var(--font-mono, ui-monospace, monospace)',
    lineHeight: '1.6',
  },
  '.cm-content': { padding: '12px 0', caretColor: 'var(--foreground)' },
  '.cm-line': { padding: '0 16px' },
  '&.cm-focused': { outline: 'none' },
  '.cm-cursor': { borderLeftColor: 'var(--foreground)' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground': {
    backgroundColor: 'color-mix(in oklab, var(--primary) 25%, transparent)',
  },
  '.cm-placeholder': { color: 'var(--muted-foreground)' },
})

export interface MarkdownEditorProps {
  value: string
  onChange: (value: string) => void
  /** Ctrl/Cmd+S */
  onSave?: () => void
  placeholder?: string
  ariaLabel: string
}

/**
 * A Markdown source editor (CodeMirror): headings, lists and tables keep
 * their shape, lines wrap, and the file is saved exactly as typed (no
 * rich-text rewriting). Uncontrolled after mount: a new `value` from
 * outside replaces the text only when it differs from what's shown.
 */
export function MarkdownEditor({
  value,
  onChange,
  onSave,
  placeholder,
  ariaLabel,
}: MarkdownEditorProps) {
  const host = useRef<HTMLDivElement>(null)
  const view = useRef<EditorView | null>(null)
  // The latest callbacks, read by the editor's listeners
  const callbacks = useRef({ onChange, onSave })
  useEffect(() => {
    callbacks.current = { onChange, onSave }
  })
  // What the editor starts with; later `value`s are applied below
  const initial = useRef({ value, placeholder, ariaLabel })

  useEffect(() => {
    if (!host.current) return
    const editor = new EditorView({
      parent: host.current,
      state: EditorState.create({
        doc: initial.current.value,
        extensions: [
          minimalSetup,
          markdown(),
          EditorView.lineWrapping,
          theme,
          hint(initial.current.placeholder ?? ''),
          EditorView.contentAttributes.of({
            'aria-label': initial.current.ariaLabel,
          }),
          keymap.of([
            {
              key: 'Mod-s',
              preventDefault: true,
              run: () => {
                callbacks.current.onSave?.()
                return true
              },
            },
          ]),
          EditorView.updateListener.of(update => {
            if (update.docChanged) {
              callbacks.current.onChange(update.state.doc.toString())
            }
          }),
        ],
      }),
    })
    view.current = editor
    return () => {
      editor.destroy()
      view.current = null
    }
  }, [])

  useEffect(() => {
    const editor = view.current
    if (!editor) return
    const shown = editor.state.doc.toString()
    if (shown !== value) {
      editor.dispatch({ changes: { from: 0, to: shown.length, insert: value } })
    }
  }, [value])

  return <div ref={host} className="h-full min-h-0 overflow-hidden" />
}
