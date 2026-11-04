import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { CircleHelp, X } from 'lucide-react'
import { toast } from 'sonner'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { WorkingDots } from '@/components/agent-console/WorkingDots'
import { MarkdownContent } from '@/components/agent-console/MarkdownContent'
import { useAsk, type AskTurn } from '@/hooks/useAsk'
import { planNeedsApproval } from '@/lib/ask'
import { client } from '@/lib/client'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import type {
  ActionPlan,
  AskContext,
  AttachmentInfo,
  EditPreview,
} from '@/lib/bindings'
import i18n from '@/i18n/config'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import { ActionPlanPreview } from './ActionPlanPreview'
import { AskAnswer } from './AskAnswer'
import { AskComposer } from './AskComposer'
import { EditPreviewCard } from './EditPreview'

/**
 * Saves a plan as a Request the core carries out, and opens its page.
 * Resolves to whether it started.
 */
async function startPlan(plan: ActionPlan, text: string): Promise<boolean> {
  const result = await client.startRequest(plan, text)
  if (result.status === 'error') {
    toast.error(result.error)
    return false
  }
  toast.success(i18n.t('ask.plan.started'))
  useNavigationStore
    .getState()
    .navigate({ kind: 'request', requestId: result.data })
  return true
}

export interface AskBoxProps {
  /** What the ask is about: the view and its focused artifact */
  context: AskContext
  placeholder: string
  /**
   * Applies an approved edit (e.g. Studio loads it into the editor);
   * without it, edit previews have no Apply button
   */
  onApplyEdit?: (preview: EditPreview) => void
  /**
   * In a conversation (an agent, a Studio chat): sends a plain chat
   * message, as the box always did. The box then offers Chat and Ask, and
   * starts in Chat.
   */
  chat?: {
    placeholder: string
    disabled?: boolean
    onSend: (text: string, attachments: AttachmentInfo[]) => void
  }
  /** Which mode a box with `chat` starts in (Chat by default) */
  defaultMode?: 'chat' | 'ask'
  /**
   * The parent decides where each message goes (Studio: chat, or Ask while
   * an artifact is open); no Chat | Ask toggle is shown
   */
  fixedMode?: 'chat' | 'ask'
  composerClassName?: string
  className?: string
}

/**
 * The universal ask box: one input for questions (answered inline),
 * actions (a plan to approve), and edits (a diff to apply). The same
 * component everywhere; only the context and placeholder change.
 */
export function AskBox({
  context,
  placeholder,
  onApplyEdit,
  chat,
  defaultMode = 'chat',
  fixedMode,
  composerClassName,
  className,
}: AskBoxProps) {
  const { t } = useTranslation()
  // A plan that only points at people or manual steps (nothing written,
  // sent or delegated) starts without asking first (work-model §5)
  const ask = useAsk(context, (plan, text, dismiss) => {
    if (plan.steps.length > 0 && !planNeedsApproval(plan)) {
      void startPlan(plan, text).then(started => {
        if (started) dismiss()
      })
    }
  })
  const [chosenMode, setMode] = useState<'chat' | 'ask'>(
    chat ? defaultMode : 'ask'
  )
  const mode = fixedMode ?? chosenMode
  const chatting = chat !== undefined && mode === 'chat'

  return (
    <div className={cn('flex w-full flex-col gap-2', className)}>
      {ask.turn && (
        <AskResult
          turn={ask.turn}
          onDismiss={ask.dismiss}
          onFollowup={question => void ask.submit(question)}
          onApplyEdit={
            onApplyEdit &&
            (preview => {
              onApplyEdit(preview)
              ask.dismiss()
            })
          }
        />
      )}
      {chat && !fixedMode && (
        <ToggleGroup
          type="single"
          size="sm"
          variant="outline"
          value={mode}
          onValueChange={value => {
            if (value === 'chat' || value === 'ask') setMode(value)
          }}
          aria-label={t('ask.mode.label')}
          className="self-start"
        >
          <ToggleGroupItem value="chat" className="h-6 px-2 text-xs">
            {t('ask.mode.chat')}
          </ToggleGroupItem>
          <ToggleGroupItem value="ask" className="h-6 px-2 text-xs">
            {t('ask.mode.ask')}
          </ToggleGroupItem>
        </ToggleGroup>
      )}
      <AskComposer
        // A fresh draft per mode
        key={mode}
        className={composerClassName}
        placeholder={chatting ? chat.placeholder : placeholder}
        disabled={chatting && chat.disabled}
        busy={!chatting && ask.pending}
        onSend={(text, attachments) => {
          if (chatting) {
            chat.onSend(text, attachments)
            return
          }
          void ask.submit(
            text,
            attachments.map(a => a.id)
          )
        }}
      />
    </div>
  )
}

/** The latest ask and what came back */
export function AskResult({
  turn,
  onDismiss,
  onFollowup,
  onApplyEdit,
}: {
  turn: AskTurn
  onDismiss: () => void
  onFollowup: (question: string) => void
  onApplyEdit?: (preview: EditPreview) => void
}) {
  const { t } = useTranslation()
  const [running, setRunning] = useState(false)

  const run = async (plan: ActionPlan) => {
    setRunning(true)
    const started = await startPlan(plan, turn.text)
    setRunning(false)
    if (started) onDismiss()
  }

  return (
    <section
      aria-label={t('ask.result')}
      aria-busy={turn.status === 'pending'}
      className="flex max-h-[50vh] flex-col gap-2 overflow-y-auto rounded-lg border bg-card p-3 shadow-xs"
    >
      <div className="flex items-start gap-2">
        <p className="min-w-0 flex-1 truncate text-xs font-medium text-muted-foreground">
          {turn.text}
        </p>
        <Button
          type="button"
          size="icon-sm"
          variant="ghost"
          className="-mt-1 -me-1 size-6"
          onClick={onDismiss}
          aria-label={t('ask.dismiss')}
          title={t('ask.dismiss')}
        >
          <X />
        </Button>
      </div>
      {turn.status === 'pending' &&
        (turn.streamed ? (
          // The answer as it streams in
          <div aria-live="polite" className="text-sm">
            <MarkdownContent content={turn.streamed} />
          </div>
        ) : (
          <p role="status" className="text-sm text-muted-foreground">
            <WorkingDots label={t('ask.thinking')} />
          </p>
        ))}
      {turn.status === 'error' && (
        <Alert variant="destructive">
          <AlertDescription>{turn.error}</AlertDescription>
        </Alert>
      )}
      {turn.response?.kind === 'question' && (
        <AskAnswer answer={turn.response.data} onFollowup={onFollowup} />
      )}
      {turn.response?.kind === 'action' && (
        <ActionPlanPreview
          plan={turn.response.data}
          busy={running}
          onApprove={
            turn.response.data.steps.length > 0
              ? () => {
                  if (turn.response?.kind === 'action') {
                    void run(turn.response.data)
                  }
                }
              : undefined
          }
        />
      )}
      {turn.response?.kind === 'edit' && (
        <EditPreviewCard
          preview={turn.response.data}
          onApply={
            onApplyEdit &&
            (() => {
              if (turn.response?.kind === 'edit') {
                onApplyEdit(turn.response.data)
              }
            })
          }
        />
      )}
      {turn.response?.kind === 'clarification' && (
        <p className="flex items-start gap-2 text-sm">
          <CircleHelp
            aria-hidden
            className="mt-0.5 size-4 shrink-0 text-muted-foreground"
          />
          {turn.response.data}
        </p>
      )}
    </section>
  )
}
