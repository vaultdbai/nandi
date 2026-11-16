import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Sparkles } from 'lucide-react'
import { DiffView } from '@/components/ask/EditPreview'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import {
  useDefaultAgentUpdates,
  useKeepDefaultAgentFile,
  useTakeDefaultAgentUpdate,
} from '@/hooks/useAgentEditor'
import type { ShippedUpdate } from '@/lib/tauri-bindings'
import { shippedFileName } from './nandi-update-model'

/**
 * On Nandi's page: the files Nandi ships that the person edited and this
 * version changed. Unedited ones were updated at startup. Each file shows
 * a diff from theirs to the new version, with Take update (the new file is
 * committed; theirs stays in git history) and Keep mine (not offered again
 * for this version). Shows nothing when there is no update.
 */
export function NandiUpdateCard({ agent }: { agent: { name: string } }) {
  const { t } = useTranslation()
  const { data: updates } = useDefaultAgentUpdates()
  if (!updates?.length) return null

  return (
    <section
      aria-labelledby="nandi-update-title"
      className="flex flex-col gap-3 border-b bg-primary/5 px-4 py-3"
    >
      <div className="flex items-start gap-2">
        <Sparkles aria-hidden className="mt-0.5 size-4 shrink-0 text-primary" />
        <div className="flex flex-col gap-0.5">
          <h2 id="nandi-update-title" className="text-sm font-medium">
            {t('agentPage.update.title', { name: agent.name })}
          </h2>
          <p className="text-xs text-muted-foreground">
            {t('agentPage.update.description')}
          </p>
        </div>
      </div>
      <ul className="flex flex-col gap-3">
        {updates.map(update => (
          <UpdateItem key={update.path} update={update} />
        ))}
      </ul>
    </section>
  )
}

/** One file's update: its name, Keep mine and Take update, and the diff */
function UpdateItem({ update }: { update: ShippedUpdate }) {
  const { t } = useTranslation()
  const take = useTakeDefaultAgentUpdate()
  const keep = useKeepDefaultAgentFile()
  const file = shippedFileName(update.path)
  const busy = take.isPending || keep.isPending

  const callbacks = (done: string) => ({
    onSuccess: () => toast.success(t(done, { file })),
    onError: (error: unknown) =>
      toast.error(t('agentPage.update.failed', { file }), {
        description: error instanceof Error ? error.message : String(error),
      }),
  })

  return (
    <li aria-label={file} className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <code className="text-xs">{file}</code>
        <div className="flex gap-2">
          <Button
            size="sm"
            variant="ghost"
            disabled={busy}
            onClick={() =>
              keep.mutate(update.path, callbacks('agentPage.update.kept'))
            }
          >
            {t('agentPage.update.keep')}
          </Button>
          <Button
            size="sm"
            disabled={busy}
            onClick={() =>
              take.mutate(update.path, callbacks('agentPage.update.taken'))
            }
          >
            {take.isPending && <Spinner />}
            {t('agentPage.update.take')}
          </Button>
        </div>
      </div>
      <DiffView diff={update.diff} />
    </li>
  )
}
