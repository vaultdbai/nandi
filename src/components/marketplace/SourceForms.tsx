import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Copy, Plus, Users } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Spinner } from '@/components/ui/spinner'
import type { CommunityRegistry } from '@/lib/bindings'
import { addRegistry, createCommunityRegistry } from '@/lib/marketplace-actions'
import {
  isMarketplaceKind as isKind,
  MARKETPLACE_KIND_LABELS as KIND_LABELS,
  type MarketplaceKind as RegistryKind,
} from './source-kinds'
import { ErrorMessage } from '@/components/ui/error-message'
import { GithubSignInPanel } from '@/components/github/GithubSignInPanel'

/** Add an agents & packs source: URL, kind, and a token or GitHub sign-in */
export function AddMarketplaceSourceForm() {
  const { t } = useTranslation()
  const [url, setUrl] = useState('')
  const [kind, setKind] = useState<RegistryKind>('community')
  const [token, setToken] = useState('')
  const [useSignIn, setUseSignIn] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  const submit = async (event: React.FormEvent) => {
    event.preventDefault()
    setSaving(true)
    setError(null)
    const failure = await addRegistry({
      url: url.trim(),
      kind,
      token: useSignIn ? null : token.trim() || null,
      useGithubSignIn: useSignIn,
    })
    setSaving(false)
    if (failure) {
      setError(failure)
    } else {
      setUrl('')
      setToken('')
    }
  }

  return (
    <Card className="shadow-none">
      <CardHeader>
        <CardTitle>{t('marketplace.addRegistry')}</CardTitle>
        <CardDescription>
          {t('marketplace.addRegistryDescription')}
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={event => void submit(event)} className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="registry-url">{t('marketplace.registryUrl')}</Label>
            <Input
              id="registry-url"
              dir="ltr"
              value={url}
              onChange={event => setUrl(event.target.value)}
              placeholder="github.com/acme/nandi-community-acme"
            />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="registry-kind">
              {t('marketplace.registryKindLabel')}
            </Label>
            <NativeSelect
              id="registry-kind"
              value={kind}
              onChange={event => {
                const value = event.target.value
                if (isKind(value)) setKind(value)
              }}
            >
              {(Object.keys(KIND_LABELS) as RegistryKind[]).map(option => (
                <NativeSelectOption key={option} value={option}>
                  {t(KIND_LABELS[option])}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </div>
          <div className="flex items-center gap-2">
            <Checkbox
              id="registry-signin"
              checked={useSignIn}
              onCheckedChange={value => setUseSignIn(value === true)}
            />
            <Label htmlFor="registry-signin" className="font-normal">
              {t('marketplace.useSignIn')}
            </Label>
          </div>
          {useSignIn && <GithubSignInPanel showSignOut={false} />}
          {!useSignIn && (
            <div className="grid gap-1.5">
              <Label htmlFor="registry-token">
                {t('marketplace.registryToken')}
              </Label>
              <Input
                id="registry-token"
                type="password"
                dir="ltr"
                autoComplete="off"
                value={token}
                onChange={event => setToken(event.target.value)}
                placeholder={t('marketplace.registryTokenPlaceholder')}
              />
            </div>
          )}
          {error && <ErrorMessage error={error} />}
          <Button type="submit" disabled={!url.trim() || saving}>
            {saving ? <Spinner /> : <Plus />}
            {t('marketplace.addRegistry')}
          </Button>
        </form>
      </CardContent>
    </Card>
  )
}

/** Create a community source and get the links to share */
export function CommunitySourceCard() {
  const { t } = useTranslation()
  const [creating, setCreating] = useState(false)
  const [result, setResult] = useState<CommunityRegistry | null>(null)

  const create = async () => {
    setCreating(true)
    setResult(await createCommunityRegistry())
    setCreating(false)
  }

  return (
    <Card className="shadow-none">
      <CardHeader>
        <CardTitle>{t('marketplace.communityTitle')}</CardTitle>
        <CardDescription>
          {t('marketplace.communityDescription')}
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-3">
        <Button
          variant="outline"
          disabled={creating}
          onClick={() => void create()}
        >
          {creating ? <Spinner /> : <Users />}
          {t('marketplace.communityCreate')}
        </Button>
        {result && (
          <div className="grid gap-2 rounded-md border bg-muted/30 p-3 text-sm">
            <p>
              {result.created
                ? t('marketplace.communityCreated')
                : t('marketplace.communityExists')}
            </p>
            <CopyField
              label={t('marketplace.communityShare')}
              value={result.registry.url}
            />
            <CopyField
              label={t('marketplace.communityInvite')}
              value={result.inviteUrl}
            />
          </div>
        )}
      </CardContent>
    </Card>
  )
}

function CopyField({ label, value }: { label: string; value: string }) {
  const { t } = useTranslation()
  const [copied, setCopied] = useState(false)
  return (
    <div className="grid gap-1">
      <span className="text-xs text-muted-foreground">{label}</span>
      <div className="flex items-center gap-2">
        <code dir="ltr" className="min-w-0 flex-1 truncate font-mono text-xs">
          {value}
        </code>
        <Button
          size="sm"
          variant="ghost"
          aria-label={t('marketplace.copy', { label })}
          onClick={() =>
            void navigator.clipboard
              .writeText(value)
              .then(() => setCopied(true))
          }
        >
          <Copy />
          {copied ? t('marketplace.copied') : t('marketplace.copyShort')}
        </Button>
      </div>
    </div>
  )
}
