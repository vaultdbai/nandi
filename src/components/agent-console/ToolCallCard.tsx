import { useTranslation } from 'react-i18next'
import { ChevronRight, CircleAlert, CircleCheck, Wrench } from 'lucide-react'
import { Spinner } from '@/components/ui/spinner'
import { cn } from '@/lib/utils'
import type { ChatMessage } from '@/types/agent'

import { askOf } from '@/components/requests/request-model'
import { formatJson } from './format-json'
import { toolLabel } from './tool-label'

type ToolCallMessage = Extract<ChatMessage, { kind: 'toolCall' }>

/** A collapsible record of one tool call: arguments, then result. */
export function ToolCallCard({ message }: { message: ToolCallMessage }) {
  const { t } = useTranslation()
  const running = message.status === 'running'

  return (
    <details className="group rounded-lg border bg-muted/20 text-sm">
      <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-2 select-none">
        <ChevronRight
          aria-hidden
          className="size-3.5 shrink-0 text-muted-foreground transition-transform group-open:rotate-90 rtl:rotate-180 rtl:group-open:rotate-90"
        />
        <Wrench
          aria-hidden
          className="size-3.5 shrink-0 text-muted-foreground"
        />
        <span className="min-w-0 truncate text-xs font-medium">
          {toolLabel(message.tool, message.args, t)}
        </span>
        <span className="ms-auto shrink-0 flex items-center gap-1.5 text-xs text-muted-foreground">
          {running ? (
            <>
              <Spinner className="size-3" />
              {t('agentConsole.toolCall.running')}
            </>
          ) : message.isError ? (
            <>
              <CircleAlert aria-hidden className="size-3.5 text-destructive" />
              {t('agentConsole.toolCall.failed')}
            </>
          ) : (
            <>
              <CircleCheck aria-hidden className="size-3.5 text-success" />
              {t('agentConsole.toolCall.done')}
            </>
          )}
        </span>
      </summary>

      <div className="space-y-2 border-t px-3 py-2">
        <code
          dir="ltr"
          className="block font-mono text-xs text-muted-foreground"
        >
          {message.tool}
        </code>
        {message.tool === 'delegate_to_agent' && (
          <AskedSection args={message.args} />
        )}
        <ToolSection label={t('agentConsole.toolCall.arguments')}>
          {formatJson(message.args)}
        </ToolSection>
        {message.result != null && (
          <ToolSection
            label={t('agentConsole.toolCall.result')}
            className={cn(message.isError && 'text-destructive')}
          >
            {formatJson(message.result)}
          </ToolSection>
        )}
      </div>
    </details>
  )
}

/** A request this agent sent: the words it asked, as the other side reads them */
function AskedSection({ args }: { args: ToolCallMessage['args'] }) {
  const { t } = useTranslation()
  const input =
    args !== null && typeof args === 'object' && !Array.isArray(args)
      ? args.input
      : undefined
  const ask = askOf(input)
  if (ask.text === null) return null
  return (
    <div>
      <p className="text-xs font-medium text-muted-foreground">
        {t('agentConsole.toolCall.asked')}
      </p>
      <blockquote className="mt-1 border-s-2 ps-3 text-sm whitespace-pre-wrap select-text">
        {ask.text}
      </blockquote>
    </div>
  )
}

function ToolSection({
  label,
  className,
  children,
}: {
  label: string
  className?: string
  children: string
}) {
  return (
    <div className="space-y-1">
      <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
        {label}
      </p>
      <pre
        dir="ltr"
        className={cn(
          'max-h-64 overflow-auto rounded-md border bg-background px-3 py-2 text-start font-mono text-xs whitespace-pre-wrap break-all',
          className
        )}
      >
        {children}
      </pre>
    </div>
  )
}
