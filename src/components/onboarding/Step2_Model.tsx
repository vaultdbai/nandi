import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQueryClient } from '@tanstack/react-query'
import { CheckCircle2, Cloud, Cpu, KeyRound, TriangleAlert } from 'lucide-react'
import { toast } from 'sonner'
import { ProviderKeyForm } from '@/components/models/ProviderKeyForm'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Spinner } from '@/components/ui/spinner'
import {
  llmKeyStatusQueryKey,
  useDefaultModel,
  useLlmKeyStatus,
  useOllama,
} from '@/hooks/useLlmKeyStatus'
import { client } from '@/lib/client'
import {
  KEYED_PROVIDERS,
  modelLabel,
  ollamaModelRef,
  providerInfo,
  providersWithKeys,
} from '@/lib/models'
import type { KeyedProvider } from '@/lib/tauri-bindings'
import { cn } from '@/lib/utils'

export interface Step2ModelProps {
  onNext: () => void
  onBack: () => void
}

type Choice = KeyedProvider | 'ollama'

/**
 * Onboarding step 2: give Nandi a model it can call. A key already in the
 * keychain (from another Nandi home on this computer) is offered in one
 * click; a typed key is tested before it's stored. Skipping needs a second
 * confirmation that says what won't work.
 */
export function Step2Model({ onNext, onBack }: Step2ModelProps) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const keys = useLlmKeyStatus()
  const defaultModel = useDefaultModel()
  const [choice, setChoice] = useState<Choice | null>(null)
  const [skipping, setSkipping] = useState(false)
  const [using, setUsing] = useState(false)

  const found = providersWithKeys(keys.data)
  const effective = defaultModel.data?.effectiveModel ?? null
  const usable = effective !== null

  /** Sets the default model and moves on */
  const applyModel = async (model: string, advance = true) => {
    setUsing(true)
    const result = await client.setDefaultModel(model)
    setUsing(false)
    if (result.status === 'error') {
      toast.error(result.error)
      return
    }
    await queryClient.invalidateQueries({ queryKey: llmKeyStatusQueryKey })
    if (advance) onNext()
  }

  const firstFound = found[0]

  return (
    <div className="flex flex-col gap-6">
      <div className="space-y-1">
        <h2 className="text-lg font-semibold">{t('onboarding.model.title')}</h2>
        <p className="text-sm text-muted-foreground">
          {t('onboarding.model.description')}
        </p>
      </div>

      {firstFound ? (
        <div
          className="flex flex-col gap-3 rounded-md border border-success/30 bg-success/5 p-3"
          data-testid="found-keys"
        >
          <p className="flex items-center gap-2 text-sm">
            <CheckCircle2
              aria-hidden
              className="size-4 shrink-0 text-success"
            />
            {t('onboarding.model.found', {
              count: found.length,
              providers: found
                .map(p => t(`models.provider.${p}.name`))
                .join(t('onboarding.model.and')),
            })}
          </p>
          <Button
            className="self-start"
            disabled={using}
            onClick={() =>
              void applyModel(providerInfo(firstFound).defaultModel)
            }
          >
            {using && <Spinner />}
            {t('onboarding.model.use', {
              provider: t(`models.provider.${firstFound}.name`),
              model: modelLabel(providerInfo(firstFound).defaultModel),
            })}
          </Button>
        </div>
      ) : (
        usable && (
          <p
            className="flex items-center gap-2 rounded-md border p-3 text-sm"
            data-testid="model-in-use"
          >
            <CheckCircle2
              aria-hidden
              className="size-4 shrink-0 text-success"
            />
            {defaultModel.data?.trial?.active
              ? t('onboarding.model.trial', { model: modelLabel(effective) })
              : t('onboarding.model.willUse', { model: modelLabel(effective) })}
          </p>
        )
      )}

      <div className="space-y-3">
        <p className="text-sm font-medium">
          {firstFound || usable
            ? t('onboarding.model.orChoose')
            : t('onboarding.model.choose')}
        </p>
        <div
          role="radiogroup"
          aria-label={t('onboarding.model.choose')}
          className="grid grid-cols-2 gap-2 sm:grid-cols-3"
        >
          {KEYED_PROVIDERS.map(({ provider }) => (
            <ProviderCard
              key={provider}
              label={t(`models.provider.${provider}.name`)}
              hint={t(`models.provider.${provider}.hint`)}
              icon={<KeyRound />}
              selected={choice === provider}
              badge={
                found.includes(provider) ? t('onboarding.model.keyFound') : null
              }
              onSelect={() => setChoice(provider)}
            />
          ))}
          <OllamaCard
            selected={choice === 'ollama'}
            onSelect={() => setChoice('ollama')}
          />
          <div className="flex flex-col gap-1 rounded-md border border-dashed p-3 text-start">
            <span className="flex items-center gap-2 text-sm font-medium [&_svg]:size-4">
              <Cloud aria-hidden />
              {t('onboarding.model.bedrock.name')}
            </span>
            <span className="text-xs text-muted-foreground">
              {t('onboarding.model.bedrock.hint')}
            </span>
          </div>
        </div>
      </div>

      {choice !== null && choice !== 'ollama' && (
        <KeyedProviderSetup
          key={choice}
          provider={choice}
          hasKey={found.includes(choice)}
          busy={using}
          onUse={model => void applyModel(model, false)}
        />
      )}
      {choice === 'ollama' && (
        <OllamaSetup
          busy={using}
          onUse={model => void applyModel(model, false)}
        />
      )}

      {skipping && (
        <div
          role="alert"
          className="flex flex-col gap-3 rounded-md border border-warning/30 bg-warning/5 p-3 text-sm"
        >
          <p className="flex items-start gap-2">
            <TriangleAlert
              aria-hidden
              className="mt-0.5 size-4 shrink-0 text-warning"
            />
            {t('onboarding.model.skipWarning')}
          </p>
          <div className="flex justify-end gap-2">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setSkipping(false)}
            >
              {t('onboarding.model.goBack')}
            </Button>
            <Button variant="outline" size="sm" onClick={onNext}>
              {t('onboarding.model.skipAnyway')}
            </Button>
          </div>
        </div>
      )}

      <div className="flex items-center justify-between gap-2">
        <Button variant="ghost" onClick={onBack}>
          {t('onboarding.back')}
        </Button>
        <div className="flex gap-2">
          {!usable && !skipping && (
            <Button variant="ghost" onClick={() => setSkipping(true)}>
              {t('onboarding.model.skip')}
            </Button>
          )}
          <Button disabled={!usable || using} onClick={onNext}>
            {t('onboarding.continue')}
          </Button>
        </div>
      </div>
    </div>
  )
}

function ProviderCard({
  label,
  hint,
  icon,
  selected,
  badge,
  disabled,
  onSelect,
}: {
  label: string
  hint: string
  icon: React.ReactNode
  selected: boolean
  badge?: string | null
  disabled?: boolean
  onSelect: () => void
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      disabled={disabled}
      onClick={onSelect}
      className={cn(
        'flex flex-col gap-1 rounded-md border p-3 text-start hover:bg-accent/40 disabled:cursor-not-allowed disabled:opacity-60',
        selected && 'border-primary ring-1 ring-primary'
      )}
    >
      <span className="flex items-center gap-2 text-sm font-medium [&_svg]:size-4">
        {icon}
        {label}
      </span>
      <span className="text-xs text-muted-foreground">{hint}</span>
      {badge && (
        <span className="flex items-center gap-1 text-xs text-success">
          <CheckCircle2 aria-hidden className="size-3" />
          {badge}
        </span>
      )}
    </button>
  )
}

function OllamaCard({
  selected,
  onSelect,
}: {
  selected: boolean
  onSelect: () => void
}) {
  const { t } = useTranslation()
  const ollama = useOllama()
  const running = ollama.data?.running ?? false
  return (
    <ProviderCard
      label={t('models.ollama.name')}
      hint={
        ollama.isPending
          ? t('models.ollama.checking')
          : running
            ? t('models.ollama.running', {
                count: ollama.data?.models.length ?? 0,
              })
            : t('models.ollama.notRunning')
      }
      icon={<Cpu />}
      selected={selected}
      onSelect={onSelect}
    />
  )
}

/** A key provider picked from the cards: its model, then its key */
function KeyedProviderSetup({
  provider,
  hasKey,
  busy,
  onUse,
}: {
  provider: KeyedProvider
  hasKey: boolean
  busy: boolean
  onUse: (model: string) => void
}) {
  const { t } = useTranslation()
  const info = providerInfo(provider)
  const [model, setModel] = useState(info.defaultModel)
  const selectId = `onboarding-model-${provider}`

  return (
    <div className="flex flex-col gap-3 rounded-md border p-3">
      <div className="space-y-1">
        <Label htmlFor={selectId}>{t('onboarding.model.defaultModel')}</Label>
        <NativeSelect
          id={selectId}
          value={model}
          onChange={event => setModel(event.target.value)}
        >
          {info.models.map(m => (
            <NativeSelectOption key={m} value={m}>
              {modelLabel(m)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </div>
      {hasKey && (
        <Button
          variant="outline"
          className="self-start"
          disabled={busy}
          onClick={() => onUse(model)}
        >
          {t('onboarding.model.useStoredKey')}
        </Button>
      )}
      <div className="space-y-1">
        <p className="text-sm font-medium">
          {t(`models.provider.${provider}.keyLabel`)}
        </p>
        <ProviderKeyForm
          provider={provider}
          submitLabel={t('models.test.button')}
          autoFocus={!hasKey}
          onSaved={() => onUse(model)}
        />
      </div>
    </div>
  )
}

function OllamaSetup({
  busy,
  onUse,
}: {
  busy: boolean
  onUse: (model: string) => void
}) {
  const { t } = useTranslation()
  const ollama = useOllama()
  const models = ollama.data?.models ?? []
  const [model, setModel] = useState<string | null>(null)
  const chosen = model ?? models[0] ?? null

  if (!ollama.data?.running) {
    return (
      <div className="flex flex-wrap items-center justify-between gap-2 rounded-md border p-3 text-sm">
        <p className="text-muted-foreground">
          {t('onboarding.model.ollamaNotRunning')}
        </p>
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
    )
  }

  if (models.length === 0) {
    return (
      <p className="rounded-md border p-3 text-sm text-muted-foreground">
        {t('onboarding.model.ollamaNoModels')}
      </p>
    )
  }

  return (
    <div className="flex flex-wrap items-end gap-2 rounded-md border p-3">
      <div className="space-y-1">
        <Label htmlFor="onboarding-ollama-model">
          {t('onboarding.model.defaultModel')}
        </Label>
        <NativeSelect
          id="onboarding-ollama-model"
          value={chosen ?? ''}
          onChange={event => setModel(event.target.value)}
        >
          {models.map(name => (
            <NativeSelectOption key={name} value={name}>
              {name}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </div>
      <Button
        disabled={busy || chosen === null}
        onClick={() => chosen && onUse(ollamaModelRef(chosen))}
      >
        {t('onboarding.model.useOllama')}
      </Button>
    </div>
  )
}
