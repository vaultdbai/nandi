import { useTranslation } from 'react-i18next'
import {
  CircleCheck,
  CircleX,
  Info,
  KeyRound,
  PlugZap,
  RefreshCw,
  Star,
  TriangleAlert,
} from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Spinner } from '@/components/ui/spinner'
import type { HarnessInfo, HarnessTestResult } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import { useAwsCredentials } from '@/hooks/useAwsAuth'
import {
  authSummary,
  runtimeStatus,
  runtimeUsable,
  type RuntimeStatus,
} from './runtime-options'

const PILL_CLASS: Record<RuntimeStatus, string> = {
  ready: 'text-success',
  notFound: 'text-muted-foreground',
  needsSignIn: 'text-warning',
  checking: 'text-muted-foreground',
  couldntCheck: 'text-destructive',
}

export function RuntimeStatusPill({ status }: { status: RuntimeStatus }) {
  const { t } = useTranslation()
  return (
    <Badge
      variant="outline"
      className={cn('gap-1', PILL_CLASS[status])}
      data-testid="runtime-status"
    >
      {status === 'checking' ? (
        <Spinner className="size-3" />
      ) : status === 'ready' ? (
        <CircleCheck />
      ) : status === 'couldntCheck' ? (
        <CircleX />
      ) : status === 'needsSignIn' ? (
        <TriangleAlert />
      ) : null}
      {t(`harness.status.${status}`)}
    </Badge>
  )
}

/**
 * One runtime. Agents run in Nandi's sandbox container, never on this
 * computer, so the card says: where it runs (is the sandbox built), what
 * it signs in with (the CLI's own sign-in here: an account, AWS Bedrock…),
 * and whether that sign-in reaches the sandbox. An API key is the last
 * resort, offered only when nothing else signs in.
 */
export function HarnessCard({
  harness,
  test,
  testing = false,
  rechecking = false,
  onTest,
  onRecheck,
  onMakeDefault,
  onDetails,
  onAddKey,
}: {
  harness: HarnessInfo
  test?: HarnessTestResult | null
  testing?: boolean
  rechecking?: boolean
  onTest?: () => void
  /** Detects again (after `claude` login or `aws sso login`) */
  onRecheck?: () => void
  onMakeDefault?: () => void
  onDetails?: () => void
  /** Opens Models on the provider whose key would sign the sandbox in */
  onAddKey?: () => void
}) {
  const { t } = useTranslation()
  const { health } = harness
  const usable = runtimeUsable(harness)
  const status = runtimeStatus(harness)
  const auth = authSummary(health.auth)
  const bedrockProfile =
    health.auth?.kind === 'bedrock' ? (health.auth.awsProfile ?? null) : null
  const aws = useAwsCredentials(bedrockProfile, health.auth?.kind === 'bedrock')
  const ssoExpired = aws.data?.expired === true

  return (
    <Card
      data-testid={`harness-${harness.id}`}
      data-settings-focus={harness.id}
    >
      <CardHeader>
        <CardTitle className="flex flex-wrap items-center gap-2">
          {harness.displayName}
          {harness.isDefault && (
            <Badge variant="secondary">
              <Star />
              {t('harness.card.default')}
            </Badge>
          )}
          <RuntimeStatusPill status={status} />
        </CardTitle>
        <CardDescription>
          {status === 'notFound'
            ? t('harness.card.notInstalled', { name: harness.displayName })
            : harness.imageAvailable
              ? t('harness.card.runsInSandbox')
              : t('harness.card.sandboxOnFirstUse')}
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-1.5 text-sm">
        {health.installed && (
          <p
            data-testid="harness-auth"
            className={cn(
              'flex items-center gap-1.5',
              !health.auth && 'text-muted-foreground'
            )}
          >
            {health.auth ? (
              <CircleCheck className="size-4 text-success" />
            ) : (
              <CircleX className="size-4" />
            )}
            {t('harness.card.signsInWith', {
              method: t(auth.key, auth.values),
            })}
          </p>
        )}
        {health.installed && (
          <p className="text-xs text-muted-foreground">
            {t('harness.card.installed', {
              version: health.version ?? t('harness.card.unknownVersion'),
            })}
          </p>
        )}
        {ssoExpired && (
          <p
            role="alert"
            data-testid="harness-sso-expired"
            className="flex flex-wrap items-center gap-1.5 text-warning"
          >
            <TriangleAlert className="size-4" />
            {t('harness.card.ssoExpired')}
            <code dir="ltr" className="font-mono text-xs">
              {bedrockProfile
                ? `aws sso login --profile ${bedrockProfile}`
                : 'aws sso login'}
            </code>
          </p>
        )}
        {usable && (
          <div
            data-testid="harness-container-auth"
            className="flex flex-wrap items-center gap-1.5"
          >
            {health.containerAuthenticated ? (
              <CircleCheck className="size-4 text-success" />
            ) : (
              <TriangleAlert className="size-4 text-warning" />
            )}
            {health.containerAuthenticated
              ? t('harness.card.sandboxSignedIn')
              : t('harness.card.sandboxNeedsSignIn')}
            {!health.containerAuthenticated && onAddKey && (
              <Button
                size="sm"
                variant="link"
                className="h-auto p-0 text-muted-foreground"
                onClick={onAddKey}
              >
                <KeyRound />
                {t('harness.card.useKeyInstead')}
              </Button>
            )}
          </div>
        )}
        {test && (
          <div>
            <Badge
              variant={test.ok ? 'secondary' : 'destructive'}
              data-testid="harness-test-result"
            >
              {test.ok ? <CircleCheck /> : <CircleX />}
              {test.ok
                ? t('harness.card.testOk')
                : t('harness.card.testFailed')}
            </Badge>
          </div>
        )}
      </CardContent>
      <CardFooter className="flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={!usable || testing}
          onClick={onTest}
        >
          {testing ? <Spinner /> : <PlugZap />}
          {t('harness.card.test')}
        </Button>
        {onRecheck && (
          <Button
            size="sm"
            variant="ghost"
            disabled={rechecking}
            onClick={() => {
              onRecheck()
              if (health.auth?.kind === 'bedrock') void aws.refetch()
            }}
          >
            {rechecking ? <Spinner /> : <RefreshCw />}
            {t('harness.card.recheck')}
          </Button>
        )}
        {!harness.isDefault && (
          <Button
            size="sm"
            variant="ghost"
            disabled={!usable}
            onClick={onMakeDefault}
          >
            <Star />
            {t('harness.card.makeDefault')}
          </Button>
        )}
        <Button size="sm" variant="ghost" onClick={onDetails}>
          <Info />
          {t('harness.card.details')}
        </Button>
      </CardFooter>
    </Card>
  )
}

/**
 * A known runtime's card before detection answers: "Checking…", then
 * "Couldn't check" with Detect again once `timedOut`
 */
export function PendingRuntimeCard({
  id,
  name,
  timedOut,
  detecting,
  onDetect,
}: {
  id: string
  name: string
  timedOut: boolean
  detecting: boolean
  onDetect: () => void
}) {
  const { t } = useTranslation()
  return (
    <Card data-testid={`harness-pending-${id}`} aria-busy={!timedOut}>
      <CardHeader>
        <CardTitle className="flex flex-wrap items-center gap-2">
          {name}
          <RuntimeStatusPill status={timedOut ? 'couldntCheck' : 'checking'} />
        </CardTitle>
      </CardHeader>
      {timedOut && (
        <CardFooter>
          <Button
            size="sm"
            variant="outline"
            disabled={detecting}
            onClick={onDetect}
          >
            {detecting ? <Spinner /> : <RefreshCw />}
            {t('harness.refresh')}
          </Button>
        </CardFooter>
      )}
    </Card>
  )
}

/** Nandi itself: always there, the default when no CLI is chosen */
export function NandiRuntimeCard({ isDefault }: { isDefault: boolean }) {
  const { t } = useTranslation()
  return (
    <Card data-testid="harness-native" data-settings-focus="native">
      <CardHeader>
        <CardTitle className="flex flex-wrap items-center gap-2">
          {t('harness.nandi.name')}
          {isDefault && (
            <Badge variant="secondary">
              <Star />
              {t('harness.card.default')}
            </Badge>
          )}
          <RuntimeStatusPill status="ready" />
        </CardTitle>
        <CardDescription>{t('harness.nandi.description')}</CardDescription>
      </CardHeader>
    </Card>
  )
}
