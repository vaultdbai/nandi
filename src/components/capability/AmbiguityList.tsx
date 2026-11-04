import { useTranslation } from 'react-i18next'
import { ArrowUpToLine, MessageCircleQuestion } from 'lucide-react'
import { toast } from 'sonner'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { errorMessage } from '@/hooks/useDelegation'
import {
  useCapabilityAmbiguities,
  usePromoteAmbiguity,
} from '@/hooks/useCapabilityEvidence'

/**
 * Clusters of clarification questions by frequency, each with samples,
 * a count, and "Promote to input schema" (which adds a field to the
 * capability's input schema, so requesters answer it up front).
 */
export function AmbiguityList({
  capabilityId,
  limit,
  canPromote = true,
}: {
  capabilityId: string
  /** Show only the most frequent `limit` clusters */
  limit?: number
  /** Only the owner can change the schema */
  canPromote?: boolean
}) {
  const { t } = useTranslation()
  const ambiguities = useCapabilityAmbiguities(capabilityId)
  const promote = usePromoteAmbiguity()
  const clusters = ambiguities.data ?? []
  const shown = limit === undefined ? clusters : clusters.slice(0, limit)

  const onPromote = (clusterId: string, label: string) =>
    promote.mutate(
      { capabilityId, clusterId },
      {
        onSuccess: () =>
          toast.success(t('capabilities.ambiguities.promoted', { label })),
        onError: error =>
          toast.error(
            t('capabilities.ambiguities.promoteFailed', {
              error: errorMessage(error),
            })
          ),
      }
    )

  if (ambiguities.isPending) return <Skeleton className="h-24" />
  if (ambiguities.error) {
    return (
      <p className="text-sm text-destructive">
        {t('capabilities.loadError', {
          error: errorMessage(ambiguities.error),
        })}
      </p>
    )
  }
  if (clusters.length === 0) {
    return (
      <Empty className="border">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <MessageCircleQuestion />
          </EmptyMedia>
          <EmptyTitle>{t('capabilities.ambiguities.emptyTitle')}</EmptyTitle>
          <EmptyDescription>
            {t('capabilities.ambiguities.emptyDescription')}
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }

  return (
    <ul
      className="flex flex-col gap-2"
      aria-label={t('capabilities.tabs.ambiguities')}
    >
      {shown.map(cluster => {
        const pending =
          promote.isPending && promote.variables?.clusterId === cluster.id
        return (
          <li
            key={cluster.id}
            data-testid={`ambiguity-${cluster.id}`}
            className="flex flex-col gap-2 rounded-md border p-3 sm:flex-row sm:items-start"
          >
            <div className="min-w-0 flex-1 space-y-1">
              <p className="text-sm font-medium">
                {t('capabilities.ambiguities.asked', {
                  label: cluster.clusterLabel,
                  count: cluster.occurrenceCount,
                })}
              </p>
              {cluster.sampleQuestions.length > 0 && (
                <ul className="list-disc space-y-0.5 ps-5 text-xs text-muted-foreground">
                  {cluster.sampleQuestions.slice(0, 3).map(question => (
                    <li key={question}>{question}</li>
                  ))}
                </ul>
              )}
            </div>
            {cluster.promotedToSchema ? (
              <Badge variant="secondary" className="shrink-0">
                {t('capabilities.ambiguities.inSchema')}
              </Badge>
            ) : (
              canPromote && (
                <Button
                  size="sm"
                  variant="outline"
                  className="shrink-0"
                  disabled={promote.isPending}
                  onClick={() => onPromote(cluster.id, cluster.clusterLabel)}
                >
                  {pending ? <Spinner /> : <ArrowUpToLine />}
                  {t('capabilities.ambiguities.promote')}
                </Button>
              )
            )}
          </li>
        )
      })}
    </ul>
  )
}
