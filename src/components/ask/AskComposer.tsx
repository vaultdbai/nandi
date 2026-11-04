import { useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ArrowUp, Camera, Paperclip } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { AttachmentChip } from '@/components/agent-console/AttachmentChip'
import { logger } from '@/lib/logger'
import { client } from '@/lib/client'
import type { AttachmentInfo } from '@/lib/tauri-bindings'
import { cn } from '@/lib/utils'

const MAX_ATTACHMENTS = 10
const MAX_TEXTAREA_HEIGHT_PX = 160

export interface AskComposerProps {
  placeholder: string
  disabled?: boolean
  /** Shows a spinner on the send button and blocks sending */
  busy?: boolean
  onSend: (text: string, attachments: AttachmentInfo[]) => void
  className?: string
  /** Focus the text box when shown (the companion's hotkey) */
  autoFocus?: boolean
}

/**
 * The text box under every ask: auto-growing textarea, attachments (desktop
 * only), Enter to send. Clears as soon as it sends.
 */
export function AskComposer({
  placeholder,
  disabled = false,
  busy = false,
  onSend,
  className,
  autoFocus = false,
}: AskComposerProps) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState('')
  // Staged in Rust; we only hold their IDs and display info
  const [attachments, setAttachments] = useState<AttachmentInfo[]>([])
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  const hasContent = draft.trim().length > 0 || attachments.length > 0
  const canSend = hasContent && !disabled && !busy

  const resize = () => {
    const el = textareaRef.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = `${Math.min(el.scrollHeight, MAX_TEXTAREA_HEIGHT_PX)}px`
  }

  const submit = () => {
    if (!canSend) return
    onSend(draft.trim(), attachments)
    setDraft('')
    setAttachments([])
    requestAnimationFrame(resize)
  }

  // The file picker runs in Rust, which copies the files and returns IDs
  const attach = async (kind: AttachmentInfo['kind']) => {
    const result = await client.pickAttachments(kind)
    if (result.status === 'error') {
      logger.warn('Attaching files failed', { error: result.error })
      toast.error(result.error)
      return
    }
    setAttachments(current =>
      [...current, ...result.data].slice(0, MAX_ATTACHMENTS)
    )
  }

  const removeAttachment = (id: string) => {
    setAttachments(current => current.filter(a => a.id !== id))
    void client.discardAttachment(id)
  }

  return (
    <form
      onSubmit={event => {
        event.preventDefault()
        submit()
      }}
      className={cn(
        'flex w-full flex-col gap-2 rounded-lg border bg-background p-2 shadow-xs transition-[color,box-shadow] focus-within:border-ring focus-within:ring-[3px] focus-within:ring-ring/50',
        className
      )}
    >
      {attachments.length > 0 && (
        <ul
          aria-label={t('composer.attachments')}
          className="flex flex-wrap gap-1.5"
        >
          {attachments.map(attachment => (
            <li key={attachment.id}>
              <AttachmentChip
                attachment={attachment}
                onRemove={removeAttachment}
              />
            </li>
          ))}
        </ul>
      )}

      <textarea
        ref={textareaRef}
        rows={1}
        autoFocus={autoFocus}
        value={draft}
        onChange={event => {
          setDraft(event.target.value)
          resize()
        }}
        onKeyDown={event => {
          // Enter sends; Shift+Enter (or an IME composition) inserts a newline
          if (
            event.key === 'Enter' &&
            !event.shiftKey &&
            !event.nativeEvent.isComposing
          ) {
            event.preventDefault()
            submit()
          }
        }}
        placeholder={placeholder}
        aria-label={placeholder}
        className="max-h-40 min-h-9 w-full resize-none bg-transparent px-1.5 py-1 text-sm outline-none placeholder:text-muted-foreground"
      />

      <div className="flex items-center gap-1">
        {client.supports.attachments && (
          <>
            <Button
              type="button"
              size="icon-sm"
              variant="ghost"
              onClick={() => void attach('file')}
              disabled={attachments.length >= MAX_ATTACHMENTS}
              aria-label={t('composer.attachFile')}
              title={t('composer.attachFile')}
            >
              <Paperclip />
            </Button>
            <Button
              type="button"
              size="icon-sm"
              variant="ghost"
              onClick={() => void attach('image')}
              disabled={attachments.length >= MAX_ATTACHMENTS}
              aria-label={t('composer.attachImage')}
              title={t('composer.attachImage')}
            >
              <Camera />
            </Button>
          </>
        )}
        <span className="ms-auto hidden text-xs text-muted-foreground sm:inline">
          {t('composer.hint')}
        </span>
        <Button
          type="submit"
          size="icon-sm"
          disabled={!canSend}
          aria-label={t('composer.send')}
          title={t('composer.send')}
          className="ms-2 rounded-full"
        >
          {busy ? <Spinner /> : <ArrowUp />}
        </Button>
      </div>
    </form>
  )
}
