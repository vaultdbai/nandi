import { cn } from '@/lib/utils'
import type { DiffLine } from './playbook-proposal-model'

/** Kept, removed and added lines, marked `+` and `-`, in reading order */
export function DiffLines({ lines }: { lines: DiffLine[] }) {
  return lines.map((line, index) => (
    <span
      key={index}
      data-kind={line.kind}
      className={cn(
        'block',
        line.kind === 'added' && 'bg-success/10 text-success',
        line.kind === 'removed' &&
          'bg-destructive/10 text-destructive line-through'
      )}
    >
      {`${line.kind === 'added' ? '+ ' : line.kind === 'removed' ? '- ' : '  '}${line.text}`}
    </span>
  ))
}
