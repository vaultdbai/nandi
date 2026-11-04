import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ExternalLink, Upload } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Spinner } from '@/components/ui/spinner'
import { GithubSignInPanel } from '@/components/github/GithubSignInPanel'
import { useMarketplaceAgents, useRegistries } from '@/hooks/useMarketplace'
import type { AgentInfo, PublishResult } from '@/lib/bindings'
import { client } from '@/lib/client'
import { publishAgent } from '@/lib/marketplace-actions'
import { nextVersion } from '@/lib/marketplace-format'

const VERSION = /^v?\d+\.\d+\.\d+$/

interface PublishDialogProps {
  agent: AgentInfo
  open: boolean
  onOpenChange: (open: boolean) => void
}

/**
 * Shares one of the user's own agents: tags the synced commit and opens a
 * pull request on a registry (merged right away on a community registry
 * the user can push to).
 */
export function PublishDialog({
  agent,
  open,
  onOpenChange,
}: PublishDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>{open && <PublishForm agent={agent} />}</DialogContent>
    </Dialog>
  )
}

function PublishForm({ agent }: { agent: AgentInfo }) {
  const { t } = useTranslation()
  const registries = useRegistries()
  const listed = useMarketplaceAgents()
  const githubRegistries = (registries.data ?? []).filter(r =>
    r.url.startsWith('github.com/')
  )
  const [registry, setRegistry] = useState<string | null>(null)
  const [version, setVersion] = useState<string | null>(null)
  const [tags, setTags] = useState('')
  const [publishing, setPublishing] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<PublishResult | null>(null)

  // Prefer a community registry; the public one needs review
  const chosen =
    registry ??
    githubRegistries.find(r => r.kind === 'community')?.url ??
    githubRegistries[0]?.url ??
    ''
  const existing = listed.data?.find(entry => entry.id === agent.id)
  const chosenVersion = version ?? nextVersion(existing?.latestVersion)
  const valid = chosen !== '' && VERSION.test(chosenVersion)

  const submit = async (event: React.FormEvent) => {
    event.preventDefault()
    if (!valid) return
    setPublishing(true)
    setError(null)
    const outcome = await publishAgent({
      agentId: agent.id,
      registryUrl: chosen,
      version: chosenVersion,
      tags: tags
        .split(',')
        .map(tag => tag.trim())
        .filter(Boolean),
    })
    setPublishing(false)
    if (typeof outcome === 'string') {
      setError(outcome)
    } else {
      setResult(outcome)
    }
  }

  if (result) {
    return (
      <div className="grid gap-4">
        <DialogHeader>
          <DialogTitle>
            {result.merged
              ? t('marketplace.publish.merged', { name: agent.name })
              : t('marketplace.publish.opened', { name: agent.name })}
          </DialogTitle>
          <DialogDescription>
            {t('marketplace.publish.resultDetail', {
              tag: result.tag,
              signed: result.signed
                ? t('marketplace.publish.signed')
                : t('marketplace.publish.unsigned'),
            })}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button onClick={() => void client.openExternal(result.prUrl)}>
            <ExternalLink />
            {t('marketplace.publish.viewPr')}
          </Button>
        </DialogFooter>
      </div>
    )
  }

  return (
    <form onSubmit={event => void submit(event)} className="grid gap-4">
      <DialogHeader>
        <DialogTitle>
          {t('marketplace.publish.title', { name: agent.name })}
        </DialogTitle>
        <DialogDescription>
          {t('marketplace.publish.description')}
        </DialogDescription>
      </DialogHeader>
      {registries.isPending ? (
        <Spinner />
      ) : githubRegistries.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t('marketplace.publish.noRegistry')}
        </p>
      ) : (
        <div className="grid gap-1.5">
          <Label htmlFor="publish-registry">
            {t('marketplace.publish.registry')}
          </Label>
          <NativeSelect
            id="publish-registry"
            value={chosen}
            onChange={event => setRegistry(event.target.value)}
          >
            {githubRegistries.map(option => (
              <NativeSelectOption key={option.url} value={option.url}>
                {option.url}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </div>
      )}
      <div className="grid gap-1.5">
        <Label htmlFor="publish-version">
          {t('marketplace.publish.version')}
        </Label>
        <Input
          id="publish-version"
          dir="ltr"
          value={chosenVersion}
          onChange={event => setVersion(event.target.value)}
          aria-invalid={!VERSION.test(chosenVersion)}
          className="font-mono"
        />
        {existing && (
          <p className="text-xs text-muted-foreground">
            {t('marketplace.publish.current', {
              version: existing.latestVersion,
            })}
          </p>
        )}
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor="publish-tags">{t('marketplace.publish.tags')}</Label>
        <Input
          id="publish-tags"
          value={tags}
          onChange={event => setTags(event.target.value)}
          placeholder={t('marketplace.publish.tagsPlaceholder')}
        />
      </div>
      <GithubSignInPanel showSignOut={false} />
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <DialogFooter>
        <Button type="submit" disabled={!valid || publishing}>
          {publishing ? <Spinner /> : <Upload />}
          {t('marketplace.publish.submit')}
        </Button>
      </DialogFooter>
    </form>
  )
}
