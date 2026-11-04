import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
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
import { Button } from '@/components/ui/button'
import {
  Field,
  FieldDescription,
  FieldError,
  FieldLabel,
} from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Skeleton } from '@/components/ui/skeleton'
import {
  branchFor,
  NAME_MAX_LENGTH,
  SLUG_MAX_LENGTH,
  validateName,
  validateSlug,
} from '@/components/onboarding/identity-model'
import type { IdentityStatus } from '@/lib/bindings'
import { errorMessage } from '@/hooks/useDelegation'
import {
  useIdentity,
  useSetDisplayName,
  useSetUserSlug,
} from '@/hooks/useIdentity'
import { useRemoteStatus } from '@/hooks/useRemote'

/** Settings › Account: the display name and handle (the only place to edit them) */
export function IdentitySection() {
  const { t } = useTranslation()
  const identity = useIdentity()

  if (identity.isPending) {
    return (
      <div className="flex flex-col gap-2" aria-busy="true">
        <Skeleton className="h-9 w-full" />
        <Skeleton className="h-9 w-full" />
      </div>
    )
  }
  if (identity.isError) {
    return (
      <p role="alert" className="text-sm text-destructive">
        {t('remote.identity.loadFailed', {
          error: errorMessage(identity.error),
        })}
      </p>
    )
  }
  // Remount when the saved identity changes, so the fields show it
  return (
    <IdentityForm
      key={`${identity.data.displayName}|${identity.data.userSlug}`}
      identity={identity.data}
    />
  )
}

function IdentityForm({ identity }: { identity: IdentityStatus }) {
  const { t } = useTranslation()
  const [name, setName] = useState(identity.displayName ?? '')
  const [slug, setSlug] = useState(identity.userSlug ?? '')
  const saveName = useSetDisplayName()
  const saveSlug = useSetUserSlug()
  const remote = useRemoteStatus()
  const backedUp = remote.data?.configured === true
  const [confirmHandle, setConfirmHandle] = useState(false)
  const nameError = validateName(name)
  const slugError = validateSlug(slug)
  const nameChanged = name.trim() !== (identity.displayName ?? '')
  const slugChanged = slug !== (identity.userSlug ?? '')

  const save = async () => {
    try {
      if (nameChanged) await saveName.mutateAsync(name)
      if (slugChanged) await saveSlug.mutateAsync(slug)
      toast.success(t('remote.identity.saved'))
    } catch (error) {
      toast.error(
        t('remote.identity.saveFailed', { error: errorMessage(error) })
      )
    }
  }

  const busy = saveName.isPending || saveSlug.isPending

  return (
    <form
      noValidate
      className="flex flex-col gap-4"
      onSubmit={e => {
        e.preventDefault()
        if (nameError || slugError) return
        // A new handle moves the backup branch: say so first
        if (slugChanged && identity.userSlug && backedUp) setConfirmHandle(true)
        else void save()
      }}
    >
      <div className="grid gap-4 sm:grid-cols-2">
        <Field data-invalid={nameError ? true : undefined}>
          <FieldLabel htmlFor="identity-name">
            {t('onboarding.name.nameLabel')}
          </FieldLabel>
          <Input
            id="identity-name"
            value={name}
            maxLength={NAME_MAX_LENGTH + 1}
            aria-invalid={nameError ? true : undefined}
            onChange={e => setName(e.target.value)}
          />
          {nameError && (
            <FieldError>
              {t(`onboarding.name.errors.${nameError}`, {
                max: NAME_MAX_LENGTH,
              })}
            </FieldError>
          )}
        </Field>
        <Field data-invalid={slugError ? true : undefined}>
          <FieldLabel htmlFor="identity-slug">
            {t('onboarding.name.handleLabel')}
          </FieldLabel>
          <Input
            id="identity-slug"
            dir="ltr"
            className="font-mono"
            value={slug}
            maxLength={SLUG_MAX_LENGTH}
            spellCheck={false}
            aria-invalid={slugError ? true : undefined}
            onChange={e => setSlug(e.target.value.toLowerCase())}
          />
          {slugError ? (
            <FieldError>
              {t(`onboarding.name.errors.handle.${slugError}`)}
            </FieldError>
          ) : (
            <FieldDescription>
              {t('settings.account.findAs', { handle: slug })}
              {backedUp && (
                <>
                  {' '}
                  {t('onboarding.branchPreview')}{' '}
                  <code dir="ltr" className="font-mono">
                    {branchFor(slug)}
                  </code>
                </>
              )}
            </FieldDescription>
          )}
        </Field>
      </div>
      {slugChanged && identity.userSlug && (
        <p className="text-xs text-muted-foreground">
          {t('remote.identity.slugChangeNote')}
        </p>
      )}
      <div className="flex justify-end">
        <Button
          type="submit"
          size="sm"
          disabled={
            busy || !!nameError || !!slugError || (!nameChanged && !slugChanged)
          }
        >
          {t('remote.identity.save')}
        </Button>
      </div>
      <AlertDialog open={confirmHandle} onOpenChange={setConfirmHandle}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('settings.account.handleConfirm.title')}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('settings.account.handleConfirm.description', {
                from: identity.branch ?? branchFor(identity.userSlug ?? ''),
                to: branchFor(slug),
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('remote.cancel')}</AlertDialogCancel>
            <AlertDialogAction onClick={() => void save()}>
              {t('settings.account.handleConfirm.confirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </form>
  )
}
