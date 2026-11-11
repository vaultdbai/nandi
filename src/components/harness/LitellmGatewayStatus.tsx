import { useTranslation } from 'react-i18next'
import { Square } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import {
  useLitellmGateway,
  useSetLitellmGateway,
} from '@/hooks/useBedrockModels'
import { rawError } from '@/lib/friendly-error'

/**
 * Whether the LiteLLM gateway runs, for which Bedrock model and where,
 * with Stop. Starting happens from the model selector.
 */
export function LitellmGatewayStatus() {
  const { t } = useTranslation()
  const gateway = useLitellmGateway()
  const setGateway = useSetLitellmGateway()

  if (gateway.isPending) return <Skeleton className="h-10 w-full" />
  const status = gateway.data
  const running = status?.running ?? false

  return (
    <div
      className="flex items-center gap-3 rounded-lg border px-3 py-2 text-sm"
      data-testid="litellm-gateway"
    >
      <Badge variant={running ? 'default' : 'outline'}>
        {running ? t('harness.gateway.running') : t('harness.gateway.stopped')}
      </Badge>
      <div className="min-w-0 flex-1">
        {running && status ? (
          <span className="truncate">
            {t('harness.gateway.serving', {
              model: status.model ?? '',
              endpoint: status.endpoint ?? '',
            })}
          </span>
        ) : (
          <span className="text-muted-foreground">
            {status?.error ?? t('harness.gateway.hint')}
          </span>
        )}
        {(gateway.error || setGateway.error) && (
          <p className="text-xs text-destructive">
            {rawError(setGateway.error ?? gateway.error)}
          </p>
        )}
      </div>
      {running && (
        <Button
          size="sm"
          variant="outline"
          disabled={setGateway.isPending}
          onClick={() => setGateway.mutate(null)}
        >
          <Square />
          {t('harness.gateway.stop')}
        </Button>
      )}
    </div>
  )
}
