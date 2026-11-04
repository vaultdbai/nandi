/**
 * Adding a capability source (Settings › Sources lists them). Nandi only
 * reads from a registry; it never pushes.
 */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Plus } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { ErrorMessage } from '@/components/ui/error-message'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Spinner } from '@/components/ui/spinner'
import { useAddCapabilityRegistry } from '@/hooks/useCapabilityRegistries'
import type { RegistryKind } from '@/lib/bindings'

const KINDS: RegistryKind[] = ['github', 'gitlab', 'local', 'generic']

/** Mirrors `RegistryKind::detect` in the core (for the hint only) */
function detectRegistryKind(url: string): RegistryKind {
  const lower = url.trim().toLowerCase()
  if (
    lower.startsWith('file://') ||
    lower.startsWith('/') ||
    lower.startsWith('~') ||
    /^[a-z]:[\\/]/.test(lower)
  ) {
    return 'local'
  }
  if (lower.includes('github.com')) return 'github'
  if (lower.includes('gitlab')) return 'gitlab'
  return 'generic'
}

/** Add a capability source: URL (kind detected from it), branch, slug */
export function AddCapabilitySourceForm() {
  const { t } = useTranslation()
  const add = useAddCapabilityRegistry()
  const [url, setUrl] = useState('')
  const [kind, setKind] = useState<RegistryKind | 'auto'>('auto')
  const [branchPrefix, setBranchPrefix] = useState('')
  const [userSlug, setUserSlug] = useState('')

  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    add.mutate(
      {
        url: url.trim(),
        kind: kind === 'auto' ? null : kind,
        branchPrefix: branchPrefix.trim() || null,
        userSlug: userSlug.trim() || null,
      },
      {
        onSuccess: () => {
          setUrl('')
          setBranchPrefix('')
          setUserSlug('')
          setKind('auto')
        },
      }
    )
  }

  const detected = detectRegistryKind(url)

  return (
    <form
      onSubmit={submit}
      className="grid gap-3 rounded-lg border p-4"
      aria-label={t('capabilityRegistries.add')}
    >
      <h4 className="text-sm font-semibold">{t('capabilityRegistries.add')}</h4>
      <div className="grid gap-1.5">
        <Label htmlFor="capability-registry-url">
          {t('capabilityRegistries.url')}
        </Label>
        <Input
          id="capability-registry-url"
          dir="ltr"
          value={url}
          onChange={event => setUrl(event.target.value)}
          placeholder={t('capabilityRegistries.urlPlaceholder')}
          required
        />
      </div>
      <div className="grid gap-3 sm:grid-cols-3">
        <div className="grid gap-1.5">
          <Label htmlFor="capability-registry-kind">
            {t('capabilityRegistries.kindLabel')}
          </Label>
          <NativeSelect
            id="capability-registry-kind"
            value={kind}
            onChange={event =>
              setKind(event.target.value as RegistryKind | 'auto')
            }
          >
            <NativeSelectOption value="auto">
              {t('capabilityRegistries.kindAuto', {
                kind: t(`capabilityRegistries.kind.${detected}`),
              })}
            </NativeSelectOption>
            {KINDS.map(option => (
              <NativeSelectOption key={option} value={option}>
                {t(`capabilityRegistries.kind.${option}`)}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="capability-registry-prefix">
            {t('capabilityRegistries.branchPrefix')}
          </Label>
          <Input
            id="capability-registry-prefix"
            dir="ltr"
            value={branchPrefix}
            onChange={event => setBranchPrefix(event.target.value)}
          />
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="capability-registry-slug">
            {t('capabilityRegistries.userSlug')}
          </Label>
          <Input
            id="capability-registry-slug"
            dir="ltr"
            value={userSlug}
            onChange={event => setUserSlug(event.target.value)}
          />
        </div>
      </div>
      <p className="text-xs text-muted-foreground">
        {t('capabilityRegistries.branchHint')}
      </p>
      {add.isError && <ErrorMessage error={add.error} />}
      <div>
        <Button type="submit" size="sm" disabled={add.isPending || !url.trim()}>
          {add.isPending ? <Spinner /> : <Plus />}
          {add.isPending
            ? t('capabilityRegistries.adding')
            : t('capabilityRegistries.add')}
        </Button>
      </div>
    </form>
  )
}
