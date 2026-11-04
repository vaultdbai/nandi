import { useTranslation } from 'react-i18next'
import { FileText, Image, X } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { AttachmentInfo } from '@/types/agent'

interface AttachmentChipProps {
  attachment: AttachmentInfo
  /** Shows a remove button when set */
  onRemove?: (id: string) => void
  className?: string
}

export function AttachmentChip({
  attachment,
  onRemove,
  className,
}: AttachmentChipProps) {
  const { t } = useTranslation()
  const Icon = attachment.kind === 'image' ? Image : FileText
  const sizeKb = Math.max(1, Math.round(attachment.sizeBytes / 1024))

  return (
    <span
      title={`${attachment.name} · ${sizeKb} KB`}
      className={cn(
        'inline-flex max-w-56 items-center gap-1.5 rounded-md border bg-muted/50 py-1 ps-2 pe-1 text-xs text-foreground',
        !onRemove && 'pe-2',
        className
      )}
    >
      <Icon aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
      <span className="truncate">{attachment.name}</span>
      {onRemove && (
        <button
          type="button"
          onClick={() => onRemove(attachment.id)}
          aria-label={t('composer.removeAttachment', { name: attachment.name })}
          className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <X className="size-3" />
        </button>
      )}
    </span>
  )
}
