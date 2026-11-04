import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQueryClient } from '@tanstack/react-query'
import { ExternalLink } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'
import { llmKeyStatusQueryKey } from '@/hooks/useLlmKeyStatus'
import { client } from '@/lib/client'
import { providerInfo } from '@/lib/models'
import type { KeyedProvider, LlmKeyTest } from '@/lib/bindings'
import { KeyTestResult } from './KeyTestResult'

export interface ProviderKeyFormProps {
  provider: KeyedProvider
  /** Text of the submit button (Settings: Save; onboarding: Test) */
  submitLabel: string
  /** Called once a key passed its test and was stored */
  onSaved?: () => void
  autoFocus?: boolean
}

/**
 * Write-only key entry: a key is tested first and stored only if it
 * works. When the provider can't be reached at all (offline), the user may
 * save it untested. Stored keys never come back to the webview.
 */
export function ProviderKeyForm({
  provider,
  submitLabel,
  onSaved,
  autoFocus,
}: ProviderKeyFormProps) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [draft, setDraft] = useState('')
  const [busy, setBusy] = useState(false)
  const [test, setTest] = useState<LlmKeyTest | null>(null)

  const store = async (apiKey: string) => {
    const saved = await client.setLlmApiKey(provider, apiKey)
    if (saved.status === 'error') {
      toast.error(saved.error)
      return
    }
    setDraft('')
    toast.success(t('models.keySaved'))
    await queryClient.invalidateQueries({ queryKey: llmKeyStatusQueryKey })
    onSaved?.()
  }

  const submit = async () => {
    const apiKey = draft.trim()
    if (!apiKey) return
    setBusy(true)
    const result = await client.testLlmKey(provider, apiKey)
    if (result.status === 'error') {
      setBusy(false)
      toast.error(result.error)
      return
    }
    setTest(result.data)
    if (result.data.ok) await store(apiKey)
    setBusy(false)
  }

  const saveUntested = async () => {
    setBusy(true)
    await store(draft.trim())
    setTest(null)
    setBusy(false)
  }

  const inputId = `llm-key-${provider}`
  const label = t(`models.provider.${provider}.keyLabel`)

  return (
    <div className="space-y-2">
      <form
        className="flex items-center gap-2"
        onSubmit={event => {
          event.preventDefault()
          void submit()
        }}
      >
        <Input
          id={inputId}
          type="password"
          dir="ltr"
          autoComplete="off"
          spellCheck={false}
          autoFocus={autoFocus}
          value={draft}
          onChange={event => {
            setDraft(event.target.value)
            setTest(null)
          }}
          placeholder={t('models.keyPlaceholder')}
          aria-label={label}
        />
        <Button type="submit" size="sm" disabled={busy || !draft.trim()}>
          {busy && <Spinner />}
          {submitLabel}
        </Button>
      </form>
      <button
        type="button"
        className="inline-flex items-center gap-1 text-xs text-muted-foreground underline-offset-2 hover:underline"
        onClick={() => void client.openExternal(providerInfo(provider).keyUrl)}
      >
        {t(`models.provider.${provider}.getKey`)}
        <ExternalLink aria-hidden className="size-3" />
      </button>
      {test && <KeyTestResult test={test} />}
      {test?.failure === 'unreachable' && draft.trim() && (
        <div className="flex flex-wrap items-center gap-2">
          <p className="text-xs text-muted-foreground">
            {t('models.saveLaterHint')}
          </p>
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => void saveUntested()}
          >
            {t('models.saveWithoutTesting')}
          </Button>
        </div>
      )}
    </div>
  )
}
