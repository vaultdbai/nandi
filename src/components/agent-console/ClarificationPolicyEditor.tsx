import { useTranslation } from 'react-i18next'
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { TagInput, type Tag } from '@/components/ui/tag-input'
import { MAX_CLARIFICATION_ROUNDS } from '@/components/agents/clarification-policy'
import type { ClarificationPolicy } from '@/lib/bindings'

const MIN_TIMEOUT_SECONDS = 10
const MAX_TIMEOUT_SECONDS = 86_400

export interface ClarificationPolicyEditorProps {
  value: ClarificationPolicy
  onChange: (value: ClarificationPolicy) => void
  disabled?: boolean
  /** Prefixes the field IDs (several editors on one page) */
  idPrefix?: string
}

const toTags = (topics: string[]): Tag[] =>
  topics.map(text => ({ id: text, text }))

const fromTags = (tags: Tag[]): string[] => [
  ...new Set(tags.map(tag => tag.text.trim()).filter(Boolean)),
]

/** An agent's clarification policy (when to ask, when to escalate), controlled */
export function ClarificationPolicyEditor({
  value,
  onChange,
  disabled = false,
  idPrefix = 'clarification-policy',
}: ClarificationPolicyEditorProps) {
  const { t } = useTranslation()
  const set = (patch: Partial<ClarificationPolicy>) =>
    onChange({ ...value, ...patch })
  const id = (name: string) => `${idPrefix}-${name}`
  const off = disabled || !value.allow

  return (
    <FieldGroup>
      <Field orientation="horizontal">
        <Switch
          id={id('allow')}
          checked={value.allow}
          disabled={disabled}
          onCheckedChange={allow => set({ allow })}
        />
        <div className="space-y-0.5">
          <FieldLabel htmlFor={id('allow')}>
            {t('tasks.policy.allow')}
          </FieldLabel>
          <FieldDescription>{t('tasks.policy.allowHint')}</FieldDescription>
        </div>
      </Field>

      <Field>
        <FieldLabel htmlFor={id('rounds')}>
          {t('tasks.policy.maxRounds', { count: value.maxRounds })}
        </FieldLabel>
        <input
          id={id('rounds')}
          type="range"
          min={0}
          max={MAX_CLARIFICATION_ROUNDS}
          step={1}
          value={value.maxRounds}
          disabled={off}
          className="w-full accent-primary"
          aria-valuetext={String(value.maxRounds)}
          onChange={e => set({ maxRounds: Number(e.target.value) })}
        />
        <FieldDescription>{t('tasks.policy.maxRoundsHint')}</FieldDescription>
      </Field>

      <Field>
        <FieldLabel htmlFor={id('auto')}>
          {t('tasks.policy.autoTopics')}
        </FieldLabel>
        <TagInput
          id={id('auto')}
          tags={toTags(value.autoTopics)}
          onTagsChange={tags => set({ autoTopics: fromTags(tags) })}
          placeholder={t('tasks.policy.topicPlaceholder')}
          disabled={off}
          aria-label={t('tasks.policy.autoTopics')}
        />
        <FieldDescription>{t('tasks.policy.autoTopicsHint')}</FieldDescription>
      </Field>

      <Field>
        <FieldLabel htmlFor={id('escalate')}>
          {t('tasks.policy.escalateTopics')}
        </FieldLabel>
        <TagInput
          id={id('escalate')}
          tags={toTags(value.escalateTopics)}
          onTagsChange={tags => set({ escalateTopics: fromTags(tags) })}
          placeholder={t('tasks.policy.topicPlaceholder')}
          disabled={off}
          aria-label={t('tasks.policy.escalateTopics')}
        />
        <FieldDescription>
          {t('tasks.policy.escalateTopicsHint')}
        </FieldDescription>
      </Field>

      <Field>
        <FieldLabel htmlFor={id('timeout')}>
          {t('tasks.policy.timeout')}
        </FieldLabel>
        <Input
          id={id('timeout')}
          type="number"
          inputMode="numeric"
          min={MIN_TIMEOUT_SECONDS}
          max={MAX_TIMEOUT_SECONDS}
          value={value.timeoutSeconds}
          disabled={off}
          className="w-32"
          onChange={e => {
            const seconds = Math.round(Number(e.target.value))
            if (Number.isFinite(seconds)) {
              set({
                timeoutSeconds: Math.min(
                  MAX_TIMEOUT_SECONDS,
                  Math.max(0, seconds)
                ),
              })
            }
          }}
        />
        <FieldDescription>{t('tasks.policy.timeoutHint')}</FieldDescription>
      </Field>
    </FieldGroup>
  )
}
