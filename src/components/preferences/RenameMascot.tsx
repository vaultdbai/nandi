import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { NandiBull } from '@/assets/brand/NandiBull'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useMascot, useRenameMascot } from '@/hooks/useMascot'
import { plainError } from '@/lib/toast-error'
import { SettingsField, SettingsSection } from './shared/SettingsComponents'

/**
 * Settings › Appearance › Mascot: the bull's name, shown wherever it
 * speaks. Shared with the CLI (`nandi rename`).
 */
export function RenameMascot() {
  const { t } = useTranslation()
  const mascot = useMascot()
  const rename = useRenameMascot()
  const [draft, setDraft] = useState<string | null>(null)
  const name = mascot.data?.name ?? 'Nandi'
  const value = draft ?? name

  const save = (next: string | null) =>
    rename.mutate(next, {
      onSuccess: info => {
        setDraft(null)
        toast.success(t('mascot.renamed', { name: info.name }))
      },
      onError: error =>
        toast.error(t('mascot.renameFailed', { error: plainError(error) })),
    })

  return (
    <SettingsSection title={t('mascot.title')}>
      <div className="flex items-center gap-4">
        <NandiBull state="rest" name={name} size={72} />
        <p className="text-sm text-muted-foreground">
          {t('mascot.description', { name })}
        </p>
      </div>
      <SettingsField label={t('mascot.nameLabel')}>
        <form
          className="flex flex-wrap items-center gap-2"
          onSubmit={event => {
            event.preventDefault()
            if (value.trim()) save(value.trim())
          }}
        >
          <Input
            className="max-w-60"
            value={value}
            maxLength={24}
            onChange={event => setDraft(event.target.value)}
            aria-label={t('mascot.nameLabel')}
          />
          <Button
            type="submit"
            size="sm"
            disabled={
              rename.isPending || !value.trim() || value.trim() === name
            }
          >
            {t('mascot.save')}
          </Button>
          {!mascot.data?.isDefault && (
            <Button
              type="button"
              size="sm"
              variant="ghost"
              disabled={rename.isPending}
              onClick={() => save(null)}
            >
              {t('mascot.reset')}
            </Button>
          )}
        </form>
      </SettingsField>
    </SettingsSection>
  )
}
