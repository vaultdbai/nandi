import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQueryClient } from '@tanstack/react-query'
import { ExternalLink, TriangleAlert } from 'lucide-react'
import { toast } from 'sonner'
import {
  llmKeyStatusQueryKey,
  useDefaultModel,
  useFreeTrialAccount,
  useLlmKeyStatus,
  useOllama,
  useProviderModels,
} from '@/hooks/useLlmKeyStatus'
import { KeyTestResult } from '@/components/models/KeyTestResult'
import { ProviderKeyForm } from '@/components/models/ProviderKeyForm'
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
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Switch } from '@/components/ui/switch'
import { Spinner } from '@/components/ui/spinner'
import { client } from '@/lib/client'
import {
  defaultModelName,
  fallbackAfterRemoving,
  joinModel,
  KEYED_PROVIDERS,
  MODEL_PROVIDERS,
  modelLabel,
  modelReason,
  modelSuggestions,
  providerNeedsKey,
  providersWithKeys,
  splitModel,
  type ModelProvider,
} from '@/lib/models'
import { cn } from '@/lib/utils'
import { authSummary } from '@/components/harness/runtime-options'
import type {
  HarnessAuth,
  KeyedProvider,
  LlmKeyTest,
  TrialAccount,
  TrialSignInProvider,
  TrialStatus,
} from '@/lib/bindings'
import { signInOptions, trialMessage, trialUsedUp } from '@/lib/free-trial'
import { SettingsField } from './shared/SettingsComponents'

/**
 * Settings › Models & runtimes: the default model and one row per provider. Keys are
 * write-only: the backend only reports whether one is configured.
 */
export function LlmKeysSection() {
  return (
    <div className="flex flex-col gap-4">
      <ModelChoiceFields />
      <ProviderKeys />
    </div>
  )
}

/** What Nandi calls: the free tokens and the default model */
export function ModelChoiceFields() {
  const { data: status } = useDefaultModel()
  return (
    <div className="flex flex-col gap-4">
      {status?.trial && <FreeTrialField trial={status.trial} />}
      <DefaultModelField />
    </div>
  )
}

/** One row per provider's key, then Ollama */
export function ProviderKeys() {
  const { data: statuses } = useLlmKeyStatus()
  const { data: status } = useDefaultModel()
  return (
    <div className="flex flex-col gap-4">
      {KEYED_PROVIDERS.map(({ provider }) => (
        <ProviderRow
          key={provider}
          provider={provider}
          configured={
            statuses?.find(s => s.provider === provider)?.configured ?? false
          }
          signIn={provider === 'anthropic' ? status?.claudeSignIn : null}
        />
      ))}
      <OllamaRow />
    </div>
  )
}

/**
 * The free tokens: Nandi's trial gateway counts them (100,000 per person,
 * shared by all their computers once they sign in). Shows what's left, a
 * switch, and sign-in to link this computer. They turn off by themselves
 * once the user has a model of their own, and can be turned back on while
 * any are left.
 */
function FreeTrialField({ trial }: { trial: TrialStatus }) {
  const { t, i18n } = useTranslation()
  const queryClient = useQueryClient()
  const [saving, setSaving] = useState(false)
  const account = useFreeTrialAccount(true)
  const format = (n: number) => n.toLocaleString(i18n.language)
  const usedUp = trialUsedUp(trial, account.data)
  const message = trialMessage(trial, account.data, account.isError)

  const toggle = async (enabled: boolean) => {
    setSaving(true)
    const result = await client.setFreeTrial(enabled)
    setSaving(false)
    if (result.status === 'error') {
      toast.error(result.error)
      return
    }
    await queryClient.invalidateQueries({ queryKey: llmKeyStatusQueryKey })
  }

  return (
    <SettingsField
      label={t('models.trial.title')}
      description={t('models.trial.description')}
    >
      <div className="flex flex-wrap items-center gap-3">
        <p className="min-w-0 flex-1 text-sm" data-testid="free-trial">
          {t(`models.trial.${message.key}`, {
            left: message.left === undefined ? '' : format(message.left),
            limit: message.limit === undefined ? '' : format(message.limit),
          })}
        </p>
        <span className="flex items-center gap-2">
          {saving && <Spinner />}
          <Label htmlFor="free-trial-switch" className="text-sm font-normal">
            {t('models.trial.turnOn')}
          </Label>
          <Switch
            id="free-trial-switch"
            checked={trial.enabled && !usedUp}
            // Used up: nothing to turn on
            disabled={saving || usedUp}
            onCheckedChange={enabled => void toggle(enabled)}
          />
        </span>
      </div>
      {account.data && <TrialSignIn account={account.data} />}
    </SettingsField>
  )
}

/** A sign-in in progress: the code to type, and how often to ask */
interface PendingSignIn {
  provider: TrialSignInProvider
  signinId: string
  userCode: string
  verificationUri: string
  interval: number
}

/**
 * "Sign in to keep your trial on all your computers": the provider's
 * device flow. Start shows a code and opens the provider's page; the app
 * asks every `interval` seconds until it's linked. Providers the gateway
 * doesn't offer yet show as coming soon.
 */
function TrialSignIn({ account }: { account: TrialAccount }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [pending, setPending] = useState<PendingSignIn | null>(null)
  const [starting, setStarting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [linked, setLinked] = useState<string | null>(null)

  // Asks again every `interval` seconds while a sign-in is pending
  useEffect(() => {
    if (!pending) return
    const timer = setTimeout(() => {
      void (async () => {
        const result = await client.pollFreeTrialSignIn(
          pending.provider,
          pending.signinId
        )
        if (result.status === 'error') {
          setPending(null)
          setError(result.error)
          return
        }
        const poll = result.data
        if (poll.status === 'linked') {
          setPending(null)
          setLinked(poll.display)
          toast.success(t('models.trial.signIn.linked', { name: poll.display }))
          await queryClient.invalidateQueries({
            queryKey: llmKeyStatusQueryKey,
          })
          return
        }
        setPending({ ...pending, interval: poll.interval })
      })()
    }, pending.interval * 1000)
    return () => clearTimeout(timer)
  }, [pending, queryClient, t])

  const start = async (provider: TrialSignInProvider) => {
    setError(null)
    setStarting(true)
    const result = await client.startFreeTrialSignIn(provider)
    setStarting(false)
    if (result.status === 'error') {
      setError(result.error)
      return
    }
    setPending({ provider, ...result.data })
    void client.openExternal(result.data.verificationUri)
  }

  if (linked || account.account) {
    return (
      <p
        className="text-sm text-muted-foreground"
        data-testid="free-trial-account"
      >
        {t('models.trial.signIn.signedIn', {
          name: linked ?? account.account,
        })}
      </p>
    )
  }

  return (
    <div className="flex flex-col gap-2" data-testid="free-trial-sign-in">
      <p className="text-sm text-muted-foreground">
        {t('models.trial.signIn.prompt')}
      </p>
      {pending ? (
        <div className="flex flex-wrap items-center gap-2 rounded-md border p-2 text-sm">
          <span>
            {t('models.trial.signIn.enterCode', {
              provider: t(`models.trial.signIn.provider.${pending.provider}`),
            })}
          </span>
          <code
            dir="ltr"
            className="rounded bg-muted px-2 py-0.5 font-mono text-base tracking-wider"
            data-testid="free-trial-user-code"
          >
            {pending.userCode}
          </code>
          <Spinner />
          <Button
            size="sm"
            variant="link"
            className="h-auto p-0"
            onClick={() => void client.openExternal(pending.verificationUri)}
          >
            {t('models.trial.signIn.openAgain')}
            <ExternalLink aria-hidden className="size-3" />
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setPending(null)}>
            {t('models.trial.signIn.cancel')}
          </Button>
        </div>
      ) : (
        <div className="flex flex-wrap gap-2">
          {signInOptions(account.signIn ?? []).map(
            ({ provider, available }) => (
              <Button
                key={provider}
                size="sm"
                variant="outline"
                disabled={!available || starting}
                onClick={() => void start(provider)}
              >
                {available
                  ? t('models.trial.signIn.with', {
                      provider: t(`models.trial.signIn.provider.${provider}`),
                    })
                  : t('models.trial.signIn.comingSoon', {
                      provider: t(`models.trial.signIn.provider.${provider}`),
                    })}
              </Button>
            )
          )}
        </div>
      )}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
    </div>
  )
}

function ProviderRow({
  provider,
  configured,
  signIn,
}: {
  provider: KeyedProvider
  configured: boolean
  /** Anthropic: the sign-in Claude calls use instead of a key */
  signIn?: HarnessAuth | null
}) {
  const { t } = useTranslation()
  const [test, setTest] = useState<LlmKeyTest | null>(null)
  const [testing, setTesting] = useState(false)
  // Keys last: with a company sign-in, the key form waits behind a link
  const [wantsKey, setWantsKey] = useState(false)
  const keyHidden = !!signIn && !configured && !wantsKey
  const method = authSummary(signIn)

  const testStored = async () => {
    setTesting(true)
    const result = await client.testLlmKey(provider, null)
    setTesting(false)
    if (result.status === 'error') {
      toast.error(result.error)
      return
    }
    setTest(result.data)
  }

  return (
    <div data-settings-focus={provider} className="rounded-md">
      <SettingsField label={t(`models.provider.${provider}.name`)}>
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="outline" className="text-muted-foreground">
            {t(configured ? 'models.keyStored' : 'models.noKey')}
          </Badge>
          {configured && (
            <>
              <Button
                size="sm"
                variant="outline"
                disabled={testing}
                onClick={() => void testStored()}
              >
                {testing && <Spinner />}
                {t('models.test.button')}
              </Button>
              <RemoveKeyButton provider={provider} />
            </>
          )}
        </div>
        {test && <KeyTestResult test={test} />}
        {signIn && (
          <p
            className="text-sm text-muted-foreground"
            data-testid="claude-sign-in"
          >
            {t('models.claudeSignIn', {
              method: t(method.key, method.values),
            })}
          </p>
        )}
        {keyHidden ? (
          <Button
            size="sm"
            variant="link"
            className="h-auto self-start p-0 text-muted-foreground"
            onClick={() => setWantsKey(true)}
          >
            {t('models.useKeyInstead')}
          </Button>
        ) : (
          <ProviderKeyForm
            provider={provider}
            submitLabel={t(configured ? 'models.replace' : 'models.save')}
            onSaved={() => setTest(null)}
          />
        )}
      </SettingsField>
    </div>
  )
}

/** Removing a key asks first and says what calls will use instead */
function RemoveKeyButton({ provider }: { provider: KeyedProvider }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const { data: statuses } = useLlmKeyStatus()
  const { data: ollama } = useOllama()
  const fallback = fallbackAfterRemoving(
    provider,
    statuses,
    ollama?.running ?? false
  )
  const providerName = t(`models.provider.${provider}.name`)

  const remove = async () => {
    const result = await client.setLlmApiKey(provider, '')
    if (result.status === 'error') {
      toast.error(result.error)
      return
    }
    toast.success(t('models.keyRemoved'))
    await queryClient.invalidateQueries({ queryKey: llmKeyStatusQueryKey })
  }

  const [open, setOpen] = useState(false)

  return (
    <>
      <Button size="sm" variant="outline" onClick={() => setOpen(true)}>
        {t('models.remove')}
      </Button>
      <AlertDialog open={open} onOpenChange={setOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('models.removeConfirm.title', { provider: providerName })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {fallback === null
                ? t('models.removeConfirm.nothingLeft')
                : t('models.removeConfirm.switchTo', {
                    provider:
                      fallback === 'ollama'
                        ? t('models.ollama.name')
                        : t(`models.provider.${fallback}.name`),
                  })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>
              {t('models.removeConfirm.cancel')}
            </AlertDialogCancel>
            <AlertDialogAction onClick={() => void remove()}>
              {t('models.removeConfirm.confirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

function OllamaRow() {
  const { t } = useTranslation()
  const ollama = useOllama()

  return (
    <SettingsField label={t('models.ollama.name')}>
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant="outline" className="text-muted-foreground">
          {ollama.isPending
            ? t('models.ollama.checking')
            : ollama.data?.running
              ? t('models.ollama.running', {
                  count: ollama.data.models.length,
                })
              : t('models.ollama.notRunning')}
        </Badge>
        <Button
          size="sm"
          variant="outline"
          disabled={ollama.isFetching}
          onClick={() => void ollama.refetch()}
        >
          {ollama.isFetching && <Spinner />}
          {t('models.ollama.checkAgain')}
        </Button>
      </div>
      <p className="text-sm text-muted-foreground">
        {ollama.data?.running ? (
          t('models.ollama.noKeyNeeded')
        ) : (
          <button
            type="button"
            className="inline-flex items-center gap-1 underline-offset-2 hover:underline"
            onClick={() => void client.openExternal('https://ollama.com')}
          >
            {t('models.ollama.install')}
            <ExternalLink aria-hidden className="size-3" />
          </button>
        )}
      </p>
    </SettingsField>
  )
}

/**
 * The model calls use first: pick the provider, then type the model. Any
 * name the provider takes works; the suggestions are only hints, since
 * models change faster than Nandi is released. Picking a provider fills
 * in its usual model (for Ollama, one it has). The line under it says
 * which model calls really use, and why, when it's another.
 */
function DefaultModelField() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const { data: status } = useDefaultModel()
  const { data: statuses } = useLlmKeyStatus()
  const { data: ollama } = useOllama()
  const installed = ollama?.running ? ollama.models : []
  const current = status?.defaultModel ?? ''
  const saved = splitModel(current)
  const [picked, setPicked] = useState<ModelProvider | null>(null)
  const [typed, setTyped] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const provider = picked ?? saved.provider ?? 'anthropic'
  const name = typed ?? saved.name
  const next = joinModel(provider, name)
  const dirty = name.trim() !== '' && next !== current
  const needsKey = providerNeedsKey(provider, statuses, !!status?.claudeSignIn)
  // The provider's own list when its key is set; else the built-in hints
  const live = useProviderModels(
    provider === 'ollama' ? null : provider,
    providersWithKeys(statuses).some(p => p === provider)
  )
  const suggestions = live.data?.length
    ? live.data
    : modelSuggestions(provider, installed)
  const reason = status
    ? modelReason({
        defaultModel: status.defaultModel,
        effectiveModel: status.effectiveModel,
        onFreeTokens: status.trial?.active === true,
      })
    : null

  const save = async () => {
    setSaving(true)
    const result = await client.setDefaultModel(next)
    setSaving(false)
    if (result.status === 'error') {
      toast.error(result.error)
      return
    }
    setPicked(null)
    setTyped(null)
    toast.success(t('models.defaultModelSaved'))
    await queryClient.invalidateQueries({ queryKey: llmKeyStatusQueryKey })
  }

  return (
    <SettingsField
      label={t('models.defaultModel')}
      description={t('models.defaultModelHint')}
    >
      <form
        className="flex flex-wrap items-center gap-2"
        onSubmit={event => {
          event.preventDefault()
          if (dirty) void save()
        }}
      >
        <NativeSelect
          aria-label={t('models.picker.provider')}
          value={provider}
          disabled={saving}
          onChange={event => {
            const chosen = MODEL_PROVIDERS.find(p => p === event.target.value)
            if (!chosen) return
            setPicked(chosen)
            setTyped(defaultModelName(chosen, installed))
          }}
        >
          {MODEL_PROVIDERS.map(p => (
            <NativeSelectOption key={p} value={p}>
              {p === 'ollama'
                ? t('models.ollama.name')
                : t(`models.provider.${p}.name`)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
        <Input
          list="llm-model-suggestions"
          dir="ltr"
          spellCheck={false}
          className="min-w-48 flex-1"
          value={name}
          placeholder={defaultModelName(provider, installed)}
          onChange={event => setTyped(event.target.value)}
          aria-label={t('models.picker.model')}
        />
        <datalist id="llm-model-suggestions">
          {suggestions.map(suggestion => (
            <option key={suggestion} value={suggestion} />
          ))}
        </datalist>
        <Button type="submit" size="sm" disabled={saving || !dirty}>
          {saving && <Spinner />}
          {t('models.save')}
        </Button>
      </form>
      {needsKey && (
        <p className="text-xs text-muted-foreground">
          {t('models.picker.needsKeyHint', {
            provider: t(`models.provider.${provider}.name`),
          })}
        </p>
      )}
      {reason && (
        <p
          role={reason.key === 'noModel' ? 'note' : undefined}
          className={cn(
            'flex items-center gap-1.5 text-sm',
            reason.key === 'noModel' ? 'text-warning' : 'text-muted-foreground'
          )}
          data-testid="model-reason"
        >
          {reason.key === 'noModel' && (
            <TriangleAlert aria-hidden className="size-4 shrink-0" />
          )}
          {reason.key === 'noModel'
            ? t('models.noModel')
            : reason.key === 'noKeyFor'
              ? t('models.using.noKeyFor', {
                  provider: t(`models.provider.${reason.provider}.name`),
                  model: modelLabel(reason.model),
                })
              : t(`models.using.${reason.key}`, {
                  model: modelLabel(reason.model),
                })}
        </p>
      )}
    </SettingsField>
  )
}
