import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { RefreshCw } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  NativeSelect,
  NativeSelectOptGroup,
  NativeSelectOption,
} from '@/components/ui/native-select'
import { Spinner } from '@/components/ui/spinner'
import {
  groupBedrockModels,
  useBedrockModels,
  useRefreshBedrockModels,
} from '@/hooks/useBedrockModels'
import { errorMessage } from '@/hooks/useDelegation'
import type { BedrockModel } from '@/lib/bindings'

function label(model: BedrockModel, t: (key: string) => string): string {
  const tested =
    model.tested === 'tested'
      ? ''
      : ` (${t(`harness.bedrock.tested.${model.tested}`)})`
  return `${model.name}${tested}`
}

/**
 * Bedrock models in two groups: Anthropic (native in Claude Code) and
 * third-party (through the LiteLLM gateway), filterable by name, ID, or
 * provider, with a refresh from Bedrock.
 */
export function BedrockModelSelector({
  value,
  onChange,
  region = null,
}: {
  value: string | null
  onChange: (modelId: string) => void
  region?: string | null
}) {
  const { t } = useTranslation()
  const models = useBedrockModels(region)
  const refresh = useRefreshBedrockModels()
  const [query, setQuery] = useState('')
  const groups = groupBedrockModels(models.data ?? [], query)
  const empty = groups.anthropic.length + groups.thirdParty.length === 0

  return (
    <div className="flex flex-col gap-2">
      <div className="flex gap-2">
        <Input
          value={query}
          onChange={event => setQuery(event.target.value)}
          placeholder={t('harness.bedrock.filter')}
          aria-label={t('harness.bedrock.filter')}
        />
        <Button
          variant="outline"
          size="icon"
          aria-label={t('harness.bedrock.refresh')}
          disabled={refresh.isPending}
          onClick={() => refresh.mutate(region)}
        >
          {refresh.isPending ? <Spinner /> : <RefreshCw />}
        </Button>
      </div>
      <NativeSelect
        aria-label={t('harness.bedrock.model')}
        value={value ?? ''}
        onChange={event => onChange(event.target.value)}
        disabled={empty}
      >
        <NativeSelectOption value="" disabled>
          {empty ? t('harness.bedrock.none') : t('harness.bedrock.choose')}
        </NativeSelectOption>
        {groups.anthropic.length > 0 && (
          <NativeSelectOptGroup label={t('harness.bedrock.anthropic')}>
            {groups.anthropic.map(model => (
              <NativeSelectOption key={model.id} value={model.id}>
                {label(model, t)}
              </NativeSelectOption>
            ))}
          </NativeSelectOptGroup>
        )}
        {groups.thirdParty.length > 0 && (
          <NativeSelectOptGroup label={t('harness.bedrock.thirdParty')}>
            {groups.thirdParty.map(model => (
              <NativeSelectOption key={model.id} value={model.id}>
                {label(model, t)}
              </NativeSelectOption>
            ))}
          </NativeSelectOptGroup>
        )}
      </NativeSelect>
      {(models.error || refresh.error) && (
        <p className="text-xs text-destructive">
          {errorMessage(refresh.error ?? models.error)}
        </p>
      )}
    </div>
  )
}
