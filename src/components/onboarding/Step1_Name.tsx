import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ChevronRight } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Field,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
} from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'
import { PROVIDER_NAMES } from '@/components/settings/email/email-model'
import {
  type MailboxIdentity,
  NAME_MAX_LENGTH,
  SLUG_MAX_LENGTH,
  slugify,
  validateName,
  validateSlug,
} from './identity-model'

export interface IdentityDraft {
  name: string
  /** The handle (the core's user slug) */
  slug: string
  /** The user typed a handle: stop deriving it from the name */
  slugEdited: boolean
}

export interface Step1NameProps {
  value: IdentityDraft
  onChange: (value: IdentityDraft) => void
  /** Called once the name and handle are valid; saves them */
  onNext: () => void
  /** The name is being saved */
  saving?: boolean
  /**
   * The connected mailbox's owner: the step then says who the user is
   * (with Change) instead of asking
   */
  mailbox?: MailboxIdentity | null
}

/**
 * Onboarding step 2: display name. The handle is derived from it and only
 * shown as a field under Customize (or when it's invalid). With a mailbox
 * connected, both come from it and the step reads "You're Dev Chechi
 * (dev@company.com) · Change".
 */
export function Step1Name({
  value,
  onChange,
  onNext,
  saving,
  mailbox,
}: Step1NameProps) {
  const { t } = useTranslation()
  const [changing, setChanging] = useState(false)
  // Errors show once the user has tried to continue or left the field
  const [touched, setTouched] = useState({ name: false, slug: false })
  const [customizing, setCustomizing] = useState(false)
  const nameError = validateName(value.name)
  const slugError = validateSlug(value.slug)
  // A name that derives no valid handle (e.g. only emoji) needs one typed
  const showHandleField =
    customizing || (touched.slug && slugError !== null && !nameError)

  const setName = (name: string) =>
    onChange({
      ...value,
      name,
      slug: value.slugEdited ? value.slug : slugify(name),
    })

  const setSlug = (slug: string) =>
    onChange({ ...value, slug: slug.toLowerCase(), slugEdited: true })

  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    setTouched({ name: true, slug: true })
    if (nameError || slugError) return
    onNext()
  }

  if (mailbox && !changing && !nameError && !slugError) {
    const details = [
      mailbox.title,
      mailbox.department,
      mailbox.manager
        ? t('onboarding.name.known.reportsTo', { manager: mailbox.manager })
        : null,
    ].filter(Boolean)
    return (
      <form onSubmit={submit} noValidate className="flex flex-col gap-6">
        <div className="space-y-1">
          <h2 className="text-lg font-semibold">
            {t('onboarding.name.known.title')}
          </h2>
          <p className="text-sm text-muted-foreground">
            {t('onboarding.name.known.description', {
              provider: PROVIDER_NAMES[mailbox.provider],
            })}
          </p>
        </div>
        <div className="space-y-1 text-sm" data-testid="known-identity">
          <p>
            {t('onboarding.name.known.youAre', {
              name: value.name.trim(),
              email: mailbox.email,
            })}{' '}
            <Button
              type="button"
              variant="link"
              size="sm"
              className="h-auto p-0"
              onClick={() => setChanging(true)}
            >
              {t('onboarding.name.known.change')}
            </Button>
          </p>
          {details.length > 0 && (
            <p className="text-muted-foreground" data-testid="known-profile">
              {details.join(' · ')}
            </p>
          )}
          <p className="text-muted-foreground" data-testid="handle-preview">
            {t('onboarding.name.handlePreview')}{' '}
            <span dir="ltr" className="font-medium text-foreground">
              @{value.slug}
            </span>
          </p>
        </div>
        <div className="flex justify-end">
          <Button type="submit" disabled={saving}>
            {saving && <Spinner />}
            {t('onboarding.continue')}
          </Button>
        </div>
      </form>
    )
  }

  return (
    <form onSubmit={submit} noValidate className="flex flex-col gap-6">
      <div className="space-y-1">
        <h2 className="text-lg font-semibold">{t('onboarding.name.title')}</h2>
        <p className="text-sm text-muted-foreground">
          {t('onboarding.name.description')}
        </p>
      </div>
      <FieldGroup>
        <Field data-invalid={touched.name && nameError ? true : undefined}>
          <FieldLabel htmlFor="onboarding-name">
            {t('onboarding.name.nameLabel')}
          </FieldLabel>
          <Input
            id="onboarding-name"
            value={value.name}
            maxLength={NAME_MAX_LENGTH + 1}
            autoFocus
            autoComplete="name"
            aria-invalid={touched.name && nameError ? true : undefined}
            placeholder={t('onboarding.name.namePlaceholder')}
            onChange={e => setName(e.target.value)}
            onBlur={() => setTouched(prev => ({ ...prev, name: true }))}
          />
          {touched.name && nameError && (
            <FieldError>
              {t(`onboarding.name.errors.${nameError}`, {
                max: NAME_MAX_LENGTH,
              })}
            </FieldError>
          )}
        </Field>

        {!showHandleField ? (
          <div className="flex flex-wrap items-center justify-between gap-2 text-sm">
            <p className="text-muted-foreground" data-testid="handle-preview">
              {t('onboarding.name.handlePreview')}{' '}
              <span dir="ltr" className="font-medium text-foreground">
                @{value.slug || '…'}
              </span>
            </p>
            <Button
              type="button"
              variant="link"
              size="sm"
              className="h-auto p-0"
              onClick={() => setCustomizing(true)}
            >
              {t('onboarding.name.customize')}
              <ChevronRight className="rtl:rotate-180" />
            </Button>
          </div>
        ) : (
          <Field data-invalid={touched.slug && slugError ? true : undefined}>
            <FieldLabel htmlFor="onboarding-slug">
              {t('onboarding.name.handleLabel')}
            </FieldLabel>
            <div dir="ltr" className="flex items-center gap-1">
              <span aria-hidden className="text-muted-foreground">
                @
              </span>
              <Input
                id="onboarding-slug"
                value={value.slug}
                maxLength={SLUG_MAX_LENGTH}
                spellCheck={false}
                autoCapitalize="off"
                autoFocus={customizing}
                aria-invalid={touched.slug && slugError ? true : undefined}
                className="font-mono"
                onChange={e => setSlug(e.target.value)}
                onBlur={() => setTouched(prev => ({ ...prev, slug: true }))}
              />
            </div>
            <FieldDescription>
              {t('onboarding.name.handleHint')}
            </FieldDescription>
            {touched.slug && slugError && (
              <FieldError>
                {t(`onboarding.name.errors.handle.${slugError}`)}
              </FieldError>
            )}
          </Field>
        )}
      </FieldGroup>
      <div className="flex justify-end">
        <Button type="submit" disabled={saving}>
          {saving && <Spinner />}
          {t('onboarding.continue')}
        </Button>
      </div>
    </form>
  )
}
