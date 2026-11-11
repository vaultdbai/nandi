import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ArrowRight, Cloud, Play, RefreshCw } from 'lucide-react'
import { toast } from 'sonner'
import { BedrockModelSelector } from '@/components/harness/BedrockModelSelector'
import {
  HarnessCard,
  NandiRuntimeCard,
  PendingRuntimeCard,
} from '@/components/harness/HarnessCard'
import { HarnessHealthDialog } from '@/components/harness/HarnessHealthDialog'
import { LitellmGatewayStatus } from '@/components/harness/LitellmGatewayStatus'
import { keyProviderFor } from '@/components/harness/runtime-options'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { useAwsCredentials, useAwsProfiles } from '@/hooks/useAwsAuth'
import {
  useBedrockModels,
  useSetLitellmGateway,
} from '@/hooks/useBedrockModels'
import { rawError } from '@/lib/friendly-error'
import {
  useDetectHarnesses,
  useHarnesses,
  useSetHarnessDefault,
  useTestHarness,
} from '@/hooks/useHarnesses'
import type { HarnessInfo, HarnessTestResult } from '@/lib/bindings'
import { openSettings } from '@/lib/settings-nav'
import { SettingsGroup } from '../SettingsFrame'

/** The runtimes Nandi knows, so their cards show before detection answers */
const KNOWN_RUNTIMES = [
  { id: 'claude-code', name: 'Claude Code' },
  { id: 'codex', name: 'Codex' },
  { id: 'gemini-cli', name: 'Gemini CLI' },
] as const

/** How long a card says "Checking…" before "Couldn't check" */
const RUNTIME_CHECK_TIMEOUT_MS = 15_000

/**
 * Nandi and the coding agents that run in its sandbox container (one
 * status pill each: the sandbox, the sign-in it uses, whether that reaches
 * the sandbox), and the Bedrock models for Claude Code. Part of Settings ›
 * Models & runtimes.
 */
export function RuntimesPanel() {
  const { t } = useTranslation()
  const harnesses = useHarnesses()
  const detect = useDetectHarnesses()
  const test = useTestHarness()
  const setDefault = useSetHarnessDefault()
  const [tests, setTests] = useState<Record<string, HarnessTestResult>>({})
  const [details, setDetails] = useState<HarnessInfo | null>(null)
  const [timedOut, setTimedOut] = useState(false)

  const pending = harnesses.isPending
  useEffect(() => {
    if (!pending) return
    const timer = window.setTimeout(
      () => setTimedOut(true),
      RUNTIME_CHECK_TIMEOUT_MS
    )
    return () => window.clearTimeout(timer)
  }, [pending])

  const runDetect = () =>
    detect.mutate(undefined, {
      onError: error => toast.error(rawError(error)),
    })

  const runTest = (harness: HarnessInfo) =>
    test.mutate(harness.id, {
      onSuccess: result => setTests(all => ({ ...all, [harness.id]: result })),
      onError: error => toast.error(rawError(error)),
    })

  const list = harnesses.data ?? []
  const nandiIsDefault = !list.some(h => h.isDefault)

  return (
    <div className="flex flex-col gap-4">
      <div className="flex justify-end">
        <Button
          size="sm"
          variant="outline"
          disabled={detect.isPending}
          onClick={runDetect}
        >
          {detect.isPending ? <Spinner /> : <RefreshCw />}
          {t('harness.refresh')}
        </Button>
      </div>
      {harnesses.error && (
        <p role="alert" className="text-sm text-destructive">
          {t('harness.detectFailed', {
            error: rawError(harnesses.error),
          })}
        </p>
      )}
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <NandiRuntimeCard isDefault={nandiIsDefault} />
        {list.length > 0
          ? list.map(harness => {
              const provider = keyProviderFor(harness.id)
              return (
                <HarnessCard
                  key={harness.id}
                  harness={harness}
                  test={tests[harness.id] ?? null}
                  testing={test.isPending && test.variables === harness.id}
                  onTest={() => runTest(harness)}
                  rechecking={detect.isPending}
                  onRecheck={runDetect}
                  onMakeDefault={() =>
                    setDefault.mutate({ harnessId: harness.id, model: null })
                  }
                  onDetails={() => setDetails(harness)}
                  onAddKey={() =>
                    openSettings(
                      'models',
                      provider ? { focus: provider } : undefined
                    )
                  }
                />
              )
            })
          : KNOWN_RUNTIMES.map(runtime => (
              <PendingRuntimeCard
                key={runtime.id}
                id={runtime.id}
                name={runtime.name}
                timedOut={timedOut || harnesses.isError}
                detecting={detect.isPending}
                onDetect={runDetect}
              />
            ))}
      </div>

      <SettingsGroup
        title={t('harness.bedrock.models')}
        description={t('harness.bedrock.description')}
      >
        <BedrockGateway />
      </SettingsGroup>

      <HarnessHealthDialog
        harness={details}
        test={details ? (tests[details.id] ?? null) : null}
        onOpenChange={open => {
          if (!open) setDetails(null)
        }}
      />
    </div>
  )
}

/** Pick a Bedrock model; third-party ones start the gateway */
function BedrockGateway() {
  const { t } = useTranslation()
  const profiles = useAwsProfiles()
  const fallback =
    profiles.data?.find(p => p.isDefault)?.name ??
    profiles.data?.[0]?.name ??
    null
  const credentials = useAwsCredentials(fallback, !!fallback)
  const signedIn = credentials.data?.valid === true
  const models = useBedrockModels(null)
  const setGateway = useSetLitellmGateway()
  const [model, setModel] = useState<string | null>(null)
  const selected = (models.data ?? []).find(m => m.id === model)

  return (
    <div className="flex flex-col gap-3 rounded-lg border p-4">
      {!signedIn && !credentials.isPending && (
        <p className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
          <Cloud className="size-4" />
          {t('harness.bedrock.needsAws')}
          <Button
            size="sm"
            variant="link"
            className="h-auto p-0"
            onClick={() => openSettings('cloud')}
          >
            {t('settings.section.cloud')}
            <ArrowRight className="rtl:rotate-180" />
          </Button>
        </p>
      )}
      <BedrockModelSelector value={model} onChange={setModel} />
      {selected && (
        <p className="text-sm text-muted-foreground">
          {selected.isAnthropic
            ? t('harness.bedrock.nativeHint')
            : t('harness.bedrock.gatewayHint')}
        </p>
      )}
      {selected?.requiresGateway && (
        <div>
          <Button
            size="sm"
            disabled={setGateway.isPending}
            onClick={() =>
              setGateway.mutate(selected.id, {
                onError: error => toast.error(rawError(error)),
              })
            }
          >
            {setGateway.isPending ? <Spinner /> : <Play />}
            {t('harness.gateway.start')}
          </Button>
        </div>
      )}
      <LitellmGatewayStatus />
    </div>
  )
}
