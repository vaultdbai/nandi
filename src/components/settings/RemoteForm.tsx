/**
 * The git remote form (URL, credential, branch prefix), shared by
 * onboarding step 3 and Settings › Backup. `RemoteFields` is the controlled
 * field set; `RemoteForm` wraps it with Save and Test for Settings.
 */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { GitBranch, ShieldAlert } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Field,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
} from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { Spinner } from '@/components/ui/spinner'
import {
  branchFor,
  DEFAULT_BRANCH_PREFIX,
} from '@/components/onboarding/identity-model'
import type { CredentialKind, RemoteStatus } from '@/lib/bindings'
import { client } from '@/lib/client'
import {
  EMPTY_REMOTE_FORM,
  toCredential,
  validateRemoteUrl,
  type RemoteFormValues,
} from './remote-form-model'
import { GithubSignInPanel } from '@/components/github/GithubSignInPanel'
import { isGithubUrl } from '@/components/github/github-url'
import { errorMessage } from '@/hooks/useDelegation'
import { useConfigureRemote } from '@/hooks/useRemote'

const CREDENTIAL_KINDS: CredentialKind[] = [
  'none',
  'github',
  'token',
  'sshKey',
  'userPass',
]

export interface RemoteFieldsProps {
  values: RemoteFormValues
  onChange: (values: RemoteFormValues) => void
  /** The user slug, for the branch preview */
  slug: string
  /** Show the URL error (after a submit attempt) */
  showErrors?: boolean
  /** Settings show the branch prefix; onboarding keeps the default */
  showPrefix?: boolean
  /** A credential is stored already: empty fields keep it */
  hasStoredCredential?: boolean
  idPrefix?: string
}

/** The remote's fields, controlled */
export function RemoteFields({
  values,
  onChange,
  slug,
  showErrors = false,
  showPrefix = false,
  hasStoredCredential = false,
  idPrefix = 'remote',
}: RemoteFieldsProps) {
  const { t } = useTranslation()
  const urlError = validateRemoteUrl(values.url)
  const github = isGithubUrl(values.url)
  // The GitHub sign-in only fits github.com addresses
  const kinds = CREDENTIAL_KINDS.filter(
    kind => kind !== 'github' || github || values.kind === 'github'
  )
  const set = (patch: Partial<RemoteFormValues>) =>
    onChange({ ...values, ...patch })
  const id = (name: string) => `${idPrefix}-${name}`

  return (
    <FieldGroup>
      <Field data-invalid={showErrors && urlError ? true : undefined}>
        <FieldLabel htmlFor={id('url')}>{t('remote.form.urlLabel')}</FieldLabel>
        <Input
          id={id('url')}
          dir="ltr"
          className="font-mono"
          placeholder="https://github.com/you/nandi-agents.git"
          value={values.url}
          spellCheck={false}
          aria-invalid={showErrors && urlError ? true : undefined}
          onChange={e => set({ url: e.target.value })}
        />
        {showErrors && urlError && (
          <FieldError>{t(`remote.form.urlErrors.${urlError}`)}</FieldError>
        )}
      </Field>

      <Field>
        <FieldLabel>{t('remote.form.credentialLabel')}</FieldLabel>
        <RadioGroup
          value={values.kind}
          onValueChange={kind => set({ kind: kind as CredentialKind })}
          aria-label={t('remote.form.credentialLabel')}
          className="grid-cols-2 sm:grid-cols-3"
        >
          {kinds.map(kind => (
            <div key={kind} className="flex items-center gap-2">
              <RadioGroupItem id={id(`kind-${kind}`)} value={kind} />
              <Label htmlFor={id(`kind-${kind}`)} className="font-normal">
                {t(`remote.form.credential.${kind}`)}
              </Label>
            </div>
          ))}
        </RadioGroup>
        {hasStoredCredential && !['none', 'github'].includes(values.kind) && (
          <FieldDescription>
            {t('remote.form.credentialStored')}
          </FieldDescription>
        )}
      </Field>

      {values.kind === 'github' && (
        <Field>
          <GithubSignInPanel showSignOut={false} />
          <FieldDescription>{t('remote.form.githubHint')}</FieldDescription>
        </Field>
      )}
      {values.kind === 'token' && (
        <Field>
          <FieldLabel htmlFor={id('token')}>
            {t('remote.form.tokenLabel')}
          </FieldLabel>
          <Input
            id={id('token')}
            type="password"
            autoComplete="off"
            value={values.token}
            onChange={e => set({ token: e.target.value })}
          />
          <FieldDescription>{t('remote.form.secretHint')}</FieldDescription>
        </Field>
      )}
      {values.kind === 'sshKey' && (
        <Field>
          <FieldLabel htmlFor={id('ssh')}>
            {t('remote.form.sshKeyLabel')}
          </FieldLabel>
          <Input
            id={id('ssh')}
            dir="ltr"
            className="font-mono"
            placeholder="~/.ssh/id_ed25519"
            value={values.sshKeyPath}
            onChange={e => set({ sshKeyPath: e.target.value })}
          />
        </Field>
      )}
      {values.kind === 'userPass' && (
        <div className="grid gap-4 sm:grid-cols-2">
          <Field>
            <FieldLabel htmlFor={id('username')}>
              {t('remote.form.usernameLabel')}
            </FieldLabel>
            <Input
              id={id('username')}
              autoComplete="off"
              value={values.username}
              onChange={e => set({ username: e.target.value })}
            />
          </Field>
          <Field>
            <FieldLabel htmlFor={id('password')}>
              {t('remote.form.passwordLabel')}
            </FieldLabel>
            <Input
              id={id('password')}
              type="password"
              autoComplete="off"
              value={values.password}
              onChange={e => set({ password: e.target.value })}
            />
          </Field>
        </div>
      )}

      {showPrefix && (
        <Field>
          <FieldLabel htmlFor={id('prefix')}>
            {t('remote.form.prefixLabel')}
          </FieldLabel>
          <Input
            id={id('prefix')}
            dir="ltr"
            className="font-mono"
            value={values.branchPrefix}
            onChange={e => set({ branchPrefix: e.target.value })}
          />
        </Field>
      )}

      <div className="space-y-2 rounded-md border bg-muted/40 p-3 text-sm">
        <p
          className="flex items-center gap-2"
          data-testid="remote-branch-preview"
        >
          <GitBranch className="size-4 shrink-0 text-muted-foreground" />
          <span>
            {t('remote.form.branchPreview')}{' '}
            <code dir="ltr" className="font-mono">
              {branchFor(slug, values.branchPrefix)}
            </code>
          </span>
        </p>
        <p className="flex items-center gap-2 text-muted-foreground">
          <ShieldAlert className="size-4 shrink-0" />
          {t('remote.form.neverMain')}
        </p>
      </div>
    </FieldGroup>
  )
}

export interface RemoteFormProps {
  /** `null` while loading */
  status: RemoteStatus | null
  onSaved?: (status: RemoteStatus) => void
  /** The user slug (branch preview, and sent with the configuration) */
  slug: string
  onCancel?: () => void
}

/** Settings: configure (or change) the remote, then optionally test it */
export function RemoteForm({
  status,
  onSaved,
  slug,
  onCancel,
}: RemoteFormProps) {
  const { t } = useTranslation()
  const config = status?.config ?? null
  const [values, setValues] = useState<RemoteFormValues>(() => ({
    ...EMPTY_REMOTE_FORM,
    url: config?.url ?? '',
    kind: config?.credentialKind ?? 'none',
    branchPrefix: config?.branchPrefix ?? DEFAULT_BRANCH_PREFIX,
  }))
  const [showErrors, setShowErrors] = useState(false)
  const [testing, setTesting] = useState(false)
  const configure = useConfigureRemote()

  const save = async (andTest: boolean) => {
    setShowErrors(true)
    if (validateRemoteUrl(values.url)) return
    let saved: RemoteStatus
    try {
      saved = await configure.mutateAsync({
        url: values.url.trim(),
        credential: toCredential(values),
        prefix: values.branchPrefix.trim() || null,
        slug: slug || null,
      })
    } catch (error) {
      toast.error(t('remote.form.saveFailed', { error: errorMessage(error) }))
      return
    }
    if (andTest) {
      setTesting(true)
      const result = await client.testRemoteConnection()
      setTesting(false)
      if (result.status === 'ok') toast.success(t('remote.testOk'))
      else toast.error(t('remote.testFailed', { error: result.error }))
    } else {
      toast.success(t('remote.form.saved'))
    }
    onSaved?.(saved)
  }

  const busy = configure.isPending || testing

  return (
    <form
      noValidate
      className="flex flex-col gap-4"
      onSubmit={e => {
        e.preventDefault()
        void save(false)
      }}
    >
      <RemoteFields
        values={values}
        onChange={setValues}
        slug={slug}
        showErrors={showErrors}
        showPrefix
        hasStoredCredential={config?.hasCredentials ?? false}
        idPrefix="settings-remote"
      />
      <div className="flex flex-wrap justify-end gap-2">
        {onCancel && (
          <Button type="button" variant="ghost" onClick={onCancel}>
            {t('remote.cancel')}
          </Button>
        )}
        <Button
          type="button"
          variant="outline"
          disabled={busy}
          onClick={() => void save(true)}
        >
          {testing && <Spinner />}
          {t('remote.form.saveAndTest')}
        </Button>
        <Button type="submit" disabled={busy}>
          {configure.isPending && !testing && <Spinner />}
          {t('remote.form.save')}
        </Button>
      </div>
    </form>
  )
}
