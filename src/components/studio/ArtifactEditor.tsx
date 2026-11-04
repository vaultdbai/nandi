import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ChevronDown, Upload } from 'lucide-react'
import { toast } from 'sonner'
import { Alert, AlertDescription } from '@/components/ui/alert'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Spinner } from '@/components/ui/spinner'
import { ArtifactLabel } from '@/components/ask/EditPreview'
import {
  useArtifact,
  useArtifactValidation,
  usePushTargets,
  useSaveArtifact,
} from '@/hooks/useStudio'
import type { ArtifactRef, PushTarget } from '@/lib/bindings'
import { useStudioStore } from '@/store/studio-store'
import { changeSummary, suggestedCommitMessage } from './change-summary'
import { AgentForm } from './AgentForm'
import { CapabilityForm } from './CapabilityForm'
import { PolicyForm } from './PolicyForm'
import { WorkflowForm } from './WorkflowForm'
import type { FormProps } from './form-fields'

/**
 * One artifact open for editing: its form over the raw file, and Save
 * (commit), Discard, and Push. An edit approved in the ask box arrives
 * pre-applied (`useStudioStore.pendingEdit`).
 */
export function ArtifactEditor({ artifact }: { artifact: ArtifactRef }) {
  const { t } = useTranslation()
  const { data: loaded, error, isPending } = useArtifact(artifact)

  if (isPending) {
    return (
      <div className="flex flex-1 items-center justify-center p-6">
        <Spinner />
      </div>
    )
  }
  if (error || !loaded) {
    return (
      <Alert variant="destructive" className="m-3">
        <AlertDescription>
          {error instanceof Error ? error.message : t('studio.loadFailed')}
        </AlertDescription>
      </Alert>
    )
  }
  // A fresh draft whenever the file on disk changes
  return (
    <Draft key={loaded.content} artifact={artifact} content={loaded.content} />
  )
}

function Draft({
  artifact,
  content,
}: {
  artifact: ArtifactRef
  content: string
}) {
  const { t } = useTranslation()
  // An edit approved in the ask box becomes the draft: when it opens the
  // file, or (the file already open) as soon as it's applied
  const pending = useStudioStore(state =>
    state.pendingEdit ? state.pendingEditFor(artifact) : null
  )
  const [draft, setDraft] = useState(() => pending?.after ?? content)
  const [adopted, setAdopted] = useState(pending)
  if (pending && pending !== adopted) {
    setAdopted(pending)
    setDraft(pending.after)
  }
  const [message, setMessage] = useState('')
  const [confirmDiscard, setConfirmDiscard] = useState(false)
  // The push target waiting on "Commit and push?"
  const [pushing, setPushing] = useState<PushTarget | null>(null)
  const [pushMessage, setPushMessage] = useState('')
  const save = useSaveArtifact()
  const { data: targets = [] } = usePushTargets()
  const dirty = draft !== content
  // Checked only once changed: the file on disk already loaded
  const { data: validation } = useArtifactValidation(
    artifact,
    dirty ? draft : null
  )
  const invalid = dirty && validation?.ok === false

  // Consumed once it's in the draft
  useEffect(() => {
    if (pending) useStudioStore.getState().clearPendingEdit(artifact)
  }, [artifact, pending])

  const changes = changeSummary(content, draft)
  const askToPush = (target: PushTarget) => {
    setPushMessage(message.trim() || suggestedCommitMessage(artifact))
    setPushing(target)
  }

  const submit = async (pushTarget: PushTarget, commitMessage = message) => {
    try {
      const result = await save.mutateAsync({
        artifact,
        content: draft,
        commitMessage,
        pushTarget,
      })
      setMessage('')
      toast.success(
        result.pushedBranch
          ? t('studio.pushed', { branch: result.pushedBranch })
          : result.commit
            ? t('studio.committed', { commit: result.commit.slice(0, 7) })
            : t('studio.saved')
      )
      for (const warning of result.warnings) toast.warning(warning)
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error))
    }
  }

  const form: FormProps = { text: draft, onChange: setDraft }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-3">
        <div className="flex items-center gap-2 text-sm font-medium">
          <ArtifactLabel artifact={artifact} />
          {dirty && <Badge variant="secondary">{t('studio.unsaved')}</Badge>}
        </div>
        <section aria-label={t('studio.fields')}>
          {artifact.kind === 'capability' && <CapabilityForm {...form} />}
          {artifact.kind === 'agent' && <AgentForm {...form} />}
          {artifact.kind === 'workflow' && <WorkflowForm {...form} />}
          {artifact.kind === 'policy' && <PolicyForm {...form} />}
          {artifact.kind === 'gateway' && <PolicyForm {...form} gateway />}
        </section>
        <div className="flex min-h-64 flex-1 flex-col gap-1.5">
          <Label htmlFor="studio-raw" className="text-xs text-muted-foreground">
            {t('studio.raw')}
          </Label>
          <textarea
            id="studio-raw"
            value={draft}
            onChange={event => setDraft(event.target.value)}
            spellCheck={false}
            className="min-h-64 flex-1 resize-none rounded-md border bg-muted/30 p-2 font-mono text-xs leading-relaxed outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50"
          />
        </div>
        {dirty && validation && (
          <ValidationNotes
            errors={validation.errors}
            warnings={validation.warnings}
          />
        )}
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-t p-3">
        <Input
          value={message}
          onChange={event => setMessage(event.target.value)}
          placeholder={t('studio.commitMessage')}
          aria-label={t('studio.commitMessage')}
          className="h-8 min-w-40 flex-1"
        />
        <Button
          size="sm"
          disabled={!dirty || invalid || save.isPending}
          onClick={() => void submit('localOnly')}
        >
          {t('studio.save')}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={!dirty || save.isPending}
          onClick={() => setConfirmDiscard(true)}
        >
          {t('studio.discard')}
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              size="sm"
              variant="outline"
              disabled={invalid || save.isPending}
            >
              <Upload />
              {t('studio.push')}
              <ChevronDown />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="max-w-72">
            {targets
              .filter(option => option.target !== 'localOnly')
              .map(option => (
                <DropdownMenuItem
                  key={option.target}
                  disabled={!option.available}
                  onSelect={() => askToPush(option.target)}
                  className="flex flex-col items-start gap-0.5"
                >
                  <span>
                    {t(`studio.pushTarget.${option.target}`, {
                      branch: option.branch ?? '',
                    })}
                  </span>
                  {option.reason && (
                    <span className="text-xs text-muted-foreground">
                      {option.reason}
                    </span>
                  )}
                </DropdownMenuItem>
              ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      <AlertDialog open={confirmDiscard} onOpenChange={setConfirmDiscard}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('studio.discardConfirm.title')}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('studio.discardConfirm.description', { ...changes })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('studio.confirm.keep')}</AlertDialogCancel>
            <AlertDialogAction onClick={() => setDraft(content)}>
              {t('studio.discard')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog
        open={pushing !== null}
        onOpenChange={open => {
          if (!open) setPushing(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {pushing &&
                t(`studio.pushTarget.${pushing}`, {
                  branch:
                    targets.find(option => option.target === pushing)?.branch ??
                    '',
                })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {dirty
                ? t('studio.pushConfirm.changes', { ...changes })
                : t('studio.pushConfirm.noChanges')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="studio-push-message">
              {t('studio.commitMessage')}
            </Label>
            <Input
              id="studio-push-message"
              value={pushMessage}
              onChange={event => setPushMessage(event.target.value)}
            />
          </div>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('studio.confirm.keep')}</AlertDialogCancel>
            <AlertDialogAction
              disabled={!pushMessage.trim()}
              onClick={() => {
                if (pushing) void submit(pushing, pushMessage.trim())
                setMessage('')
              }}
            >
              {t('studio.pushConfirm.confirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

/** Why the draft can't be saved (errors), or what to look at (warnings) */
function ValidationNotes({
  errors,
  warnings,
}: {
  errors: string[]
  warnings: string[]
}) {
  const { t } = useTranslation()
  if (errors.length === 0 && warnings.length === 0) {
    return (
      <p role="status" className="text-xs text-success">
        {t('studio.validation.valid')}
      </p>
    )
  }
  return (
    <div className="space-y-1 text-xs" data-testid="artifact-validation">
      {errors.length > 0 && (
        <p role="alert" className="font-medium text-destructive">
          {t('studio.validation.notValid')}
        </p>
      )}
      <ul className="list-disc space-y-0.5 ps-4">
        {errors.map(error => (
          <li key={error} className="text-destructive">
            {error}
          </li>
        ))}
        {warnings.map(warning => (
          <li key={warning} className="text-warning">
            {warning}
          </li>
        ))}
      </ul>
    </div>
  )
}
