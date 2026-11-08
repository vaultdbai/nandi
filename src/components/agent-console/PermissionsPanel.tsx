/**
 * An agent's real capabilities, as declared in its skill.md: the tools it
 * may use and which need approval, the hosts it may reach, and its pilots
 * with the capabilities the user granted (each can be switched off here).
 *
 * Capabilities come from the agent's definition (`loadEntity`): the agent
 * snapshot doesn't carry them. Grants are machine-local consent, read with
 * `agentPilotPermissions`.
 *
 * A browser pilot also shows the agent's sites and accounts: the secrets it
 * types (set or remove a value, add a new one for an agent made here; the
 * values go to the keychain and are never shown) and Sign out everywhere,
 * which clears its profile.
 */
import { useTranslation } from 'react-i18next'
import type { TFunction } from 'i18next'
import { toast } from 'sonner'
import {
  Globe,
  LogOut,
  MonitorSmartphone,
  Pencil,
  Plus,
  Wrench,
} from 'lucide-react'
import { Alert, AlertDescription } from '@/components/ui/alert'
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
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Switch } from '@/components/ui/switch'
import { useAgentDefinition, usePilotDefinitions } from '@/hooks/useAgentEditor'
import {
  useAddPilotSecret,
  useAgentPilotPermissions,
  useClearBrowserProfile,
  useSetPilotGrant,
  useSetPilotSecret,
} from '@/hooks/usePilots'
import { Input } from '@/components/ui/input'
import { useState } from 'react'
import { RUNTIME_TOOLS } from '@/lib/marketplace-capabilities'
import { openAgentInStudio } from '@/lib/ask'
import { normalizeSecretName, secretNameProblem } from './site-secrets-model'

type Level = 'allowed' | 'ask' | 'denied'

/** Built-in tools have plain names (`read_file` → Read files) */
const BUILTIN_TOOLS = [
  'shell',
  'read_file',
  'write_file',
  'list_dir',
  'delegate_to_agent',
]

function toolName(t: TFunction, tool: string): string {
  return BUILTIN_TOOLS.includes(tool) ? t(`agentPage.tool.${tool}`) : tool
}

const LEVEL_CLASSES: Record<Level, string> = {
  allowed: 'border-success/30 bg-success/10 text-success',
  ask: 'border-warning/30 bg-warning/10 text-warning',
  denied: 'border-destructive/30 bg-destructive/10 text-destructive',
}

export function PermissionsPanel({ agentId }: { agentId: string }) {
  const { t } = useTranslation()
  const definition = useAgentDefinition(agentId)

  if (definition.isPending) {
    return (
      <div className="flex flex-col gap-2 p-4">
        <Skeleton className="h-16 w-full" />
        <Skeleton className="h-16 w-full" />
      </div>
    )
  }
  if (definition.isError) {
    return (
      <div className="p-4">
        <Alert variant="destructive">
          <AlertDescription>{definition.error.message}</AlertDescription>
        </Alert>
      </div>
    )
  }

  const agent = definition.data
  const tools = agent.allowedTools ?? [...RUNTIME_TOOLS]
  const level = (tool: string): Level =>
    agent.requiresApproval.includes(tool) ? 'ask' : 'allowed'

  return (
    <ScrollArea className="h-full">
      <div className="flex flex-col gap-5 p-4">
        <div className="flex items-start justify-between gap-2">
          <p className="text-xs text-muted-foreground">
            {t('agentConsole.context.permissions.description')}
          </p>
          {!agent.readOnly && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => openAgentInStudio(agentId)}
            >
              <Pencil />
              {t('agentPage.editInStudio')}
            </Button>
          )}
        </div>

        <Section icon={<Wrench />} title={t('editor.permissions.tools')}>
          {agent.allowedTools === null && (
            <Note>{t('editor.permissions.allTools')}</Note>
          )}
          {tools.length === 0 && <Note>{t('editor.permissions.noTools')}</Note>}
          {tools.map(tool => (
            <Row
              key={tool}
              title={toolName(t, tool)}
              mono={!BUILTIN_TOOLS.includes(tool)}
              tooltip={tool}
            >
              <LevelBadge level={level(tool)} />
            </Row>
          ))}
        </Section>

        <Section title={t('editor.permissions.approvals')}>
          {agent.requiresApproval.length === 0 ? (
            <Note>{t('editor.permissions.noApprovals')}</Note>
          ) : (
            <Row
              title={agent.requiresApproval
                .map(tool => toolName(t, tool))
                .join(', ')}
            >
              <LevelBadge level="ask" />
            </Row>
          )}
        </Section>

        <Section icon={<Globe />} title={t('editor.permissions.network')}>
          {agent.networkRules.length === 0 ? (
            <Row title={t('editor.permissions.noNetwork')}>
              <LevelBadge level="denied" />
            </Row>
          ) : (
            agent.networkRules.map(rule => (
              <Row
                key={rule.host}
                title={rule.host}
                subtitle={
                  rule.methods?.length
                    ? rule.methods.join(', ')
                    : t('editor.permissions.anyMethod')
                }
                mono
              >
                <LevelBadge level="allowed" />
              </Row>
            ))
          )}
        </Section>

        <PilotsSection agentId={agentId} readOnly={agent.readOnly} />
      </div>
    </ScrollArea>
  )
}

function PilotsSection({
  agentId,
  readOnly,
}: {
  agentId: string
  readOnly: boolean
}) {
  const { t } = useTranslation()
  const pilots = usePilotDefinitions()
  const permissions = useAgentPilotPermissions(agentId)
  const setGrant = useSetPilotGrant(agentId)
  const declared = permissions.data ?? []

  const handleToggle = (
    pilotId: string,
    capability: string,
    granted: boolean
  ) => {
    setGrant.mutate(
      { pilotId, capability, granted },
      {
        onError: error =>
          toast.error(
            t(
              granted
                ? 'editor.permissions.grantFailed'
                : 'editor.permissions.revokeFailed'
            ),
            {
              description: String(
                error instanceof Error ? error.message : error
              ),
            }
          ),
      }
    )
  }

  return (
    <Section
      icon={<MonitorSmartphone />}
      title={t('editor.permissions.pilots')}
    >
      {permissions.isError && (
        <Note>{t('editor.permissions.pilotsFailed')}</Note>
      )}
      {permissions.isSuccess && declared.length === 0 && (
        <Note>{t('editor.permissions.noPilots')}</Note>
      )}
      {declared.map(permission => {
        const name =
          pilots.data?.find(p => p.id === permission.pilotId)?.name ??
          permission.pilotId
        if (!permission.available) {
          return (
            <Row key={permission.pilotId} title={permission.pilotId} mono>
              <Badge variant="outline">
                {t('editor.permissions.notOnThisOs')}
              </Badge>
            </Row>
          )
        }
        return [
          ...permission.capabilities.map(capability => {
            const granted = permission.granted.includes(capability)
            return (
              <Row
                key={`${permission.pilotId}/${capability}`}
                title={name}
                subtitle={capability}
              >
                <Switch
                  aria-label={`${name} ${capability}`}
                  checked={granted}
                  disabled={setGrant.isPending}
                  onCheckedChange={on =>
                    handleToggle(permission.pilotId, capability, on)
                  }
                />
              </Row>
            )
          }),
          (permission.allows ?? []).length > 0 && (
            <Row
              key={`${permission.pilotId}/allows`}
              title={t('editor.permissions.pilotAuto')}
              subtitle={(permission.allows ?? [])
                .map(kind => t(`editor.permissions.pilotAllow.${kind}`))
                .join(', ')}
            >
              <LevelBadge level="allowed" />
            </Row>
          ),
          permission.browser && (
            <Note key={`${permission.pilotId}/sites`}>
              {t('editor.permissions.sitesHint', { name })}
            </Note>
          ),
          ...(permission.secrets ?? []).map(secret => (
            <PilotSecretRow
              key={`${permission.pilotId}/secret/${secret.name}`}
              agentId={agentId}
              pilotId={permission.pilotId}
              name={secret.name}
              isSet={secret.isSet}
            />
          )),
          permission.browser && !readOnly && (
            <AddSiteSecretRow
              key={`${permission.pilotId}/add-secret`}
              agentId={agentId}
              pilotId={permission.pilotId}
              existing={(permission.secrets ?? []).map(s => s.name)}
            />
          ),
          permission.browser && (
            <SignOutRow
              key={`${permission.pilotId}/sign-out`}
              agentId={agentId}
              pilotId={permission.pilotId}
              pilotName={name}
              hasProfile={permission.hasProfile ?? false}
            />
          ),
        ]
      })}
    </Section>
  )
}

/** A secret the browser types for the agent: write-only, never shown */
function PilotSecretRow({
  agentId,
  pilotId,
  name,
  isSet,
}: {
  agentId: string
  pilotId: string
  name: string
  isSet: boolean
}) {
  const { t } = useTranslation()
  const save = useSetPilotSecret(agentId)
  const [value, setValue] = useState('')
  const submit = (next: string) =>
    save.mutate(
      { pilotId, name, value: next },
      {
        onSuccess: () => {
          setValue('')
          toast.success(
            t(
              next
                ? 'editor.permissions.secretSaved'
                : 'editor.permissions.secretRemoved'
            )
          )
        },
        onError: error =>
          toast.error(t('editor.permissions.secretFailed'), {
            description: String(error instanceof Error ? error.message : error),
          }),
      }
    )
  return (
    <Row
      title={name}
      subtitle={t(
        isSet
          ? 'editor.permissions.secretIsSet'
          : 'editor.permissions.secretNotSet'
      )}
      mono
    >
      <form
        className="flex items-center gap-1"
        onSubmit={event => {
          event.preventDefault()
          if (value) submit(value)
        }}
      >
        <Input
          type="password"
          autoComplete="off"
          aria-label={t('editor.permissions.secretValue', { name })}
          className="h-8 w-36"
          value={value}
          placeholder={isSet ? '••••••' : ''}
          onChange={event => setValue(event.target.value)}
        />
        <Button size="sm" type="submit" disabled={!value || save.isPending}>
          {t('editor.permissions.secretSave')}
        </Button>
        {isSet && (
          <Button
            size="sm"
            type="button"
            variant="ghost"
            disabled={save.isPending}
            onClick={() => submit('')}
          >
            {t('editor.permissions.secretClear')}
          </Button>
        )}
      </form>
    </Row>
  )
}

/** A new secret for the agent's browser: the name into skill.md, the value
 * into the keychain (cleared from the form once saved) */
function AddSiteSecretRow({
  agentId,
  pilotId,
  existing,
}: {
  agentId: string
  pilotId: string
  existing: string[]
}) {
  const { t } = useTranslation()
  const add = useAddPilotSecret(agentId)
  const [name, setName] = useState('')
  const [value, setValue] = useState('')
  const problem = name ? secretNameProblem(name, existing) : null
  const submit = () =>
    add.mutate(
      { pilotId, name, value },
      {
        onSuccess: () => {
          setName('')
          setValue('')
          toast.success(t('editor.permissions.secretAdded', { name }))
        },
        onError: error =>
          toast.error(t('editor.permissions.secretFailed'), {
            description: String(error instanceof Error ? error.message : error),
          }),
      }
    )
  return (
    <li className="flex flex-col gap-1 px-3 py-2.5">
      <form
        className="flex flex-wrap items-center gap-1"
        onSubmit={event => {
          event.preventDefault()
          if (!secretNameProblem(name, existing) && value) submit()
        }}
      >
        <Input
          dir="ltr"
          autoComplete="off"
          aria-label={t('editor.permissions.newSecretName')}
          className="h-8 w-44 font-mono text-xs"
          placeholder="ACME_PASSWORD"
          value={name}
          onChange={event => setName(normalizeSecretName(event.target.value))}
        />
        <Input
          type="password"
          autoComplete="new-password"
          aria-label={t('editor.permissions.newSecretValue')}
          className="h-8 w-36"
          value={value}
          onChange={event => setValue(event.target.value)}
        />
        <Button
          size="sm"
          type="submit"
          variant="outline"
          disabled={!name || !!problem || !value || add.isPending}
        >
          <Plus />
          {t('editor.permissions.addSecret')}
        </Button>
      </form>
      {problem && (
        <p role="alert" className="text-xs text-destructive">
          {t(`editor.permissions.secretName.${problem}`)}
        </p>
      )}
    </li>
  )
}

/** Sign out everywhere: stops the agent's browser and clears its profile */
function SignOutRow({
  agentId,
  pilotId,
  pilotName,
  hasProfile,
}: {
  agentId: string
  pilotId: string
  pilotName: string
  hasProfile: boolean
}) {
  const { t } = useTranslation()
  const clear = useClearBrowserProfile(agentId)
  const [confirming, setConfirming] = useState(false)
  return (
    <Row
      title={t('editor.permissions.signOutTitle')}
      subtitle={t(
        hasProfile
          ? 'editor.permissions.signOutHasProfile'
          : 'editor.permissions.signOutNoProfile'
      )}
    >
      <Button
        size="sm"
        variant="outline"
        disabled={!hasProfile || clear.isPending}
        onClick={() => setConfirming(true)}
      >
        <LogOut />
        {t('editor.permissions.signOut')}
      </Button>
      <AlertDialog open={confirming} onOpenChange={setConfirming}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('editor.permissions.signOutConfirmTitle', {
                name: pilotName,
              })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('editor.permissions.signOutConfirmDescription')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>
              {t('editor.permissions.signOutCancel')}
            </AlertDialogCancel>
            <AlertDialogAction
              onClick={() =>
                clear.mutate(pilotId, {
                  onSuccess: () =>
                    toast.success(t('editor.permissions.signedOut')),
                  onError: error =>
                    toast.error(t('editor.permissions.signOutFailed'), {
                      description: String(
                        error instanceof Error ? error.message : error
                      ),
                    }),
                })
              }
            >
              {t('editor.permissions.signOut')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Row>
  )
}

function LevelBadge({ level }: { level: Level }) {
  const { t } = useTranslation()
  return (
    <Badge variant="outline" className={LEVEL_CLASSES[level]}>
      {t(`agentConsole.context.permissions.level.${level}`)}
    </Badge>
  )
}

function Section({
  icon,
  title,
  children,
}: {
  icon?: React.ReactNode
  title: string
  children: React.ReactNode
}) {
  return (
    <section className="flex flex-col gap-2">
      <h3 className="flex items-center gap-1.5 text-xs font-medium tracking-wide text-muted-foreground uppercase [&_svg]:size-3.5">
        {icon}
        {title}
      </h3>
      <ul className="flex flex-col divide-y rounded-lg border">{children}</ul>
    </section>
  )
}

function Note({ children }: { children: React.ReactNode }) {
  return <li className="px-3 py-2 text-xs text-muted-foreground">{children}</li>
}

function Row({
  title,
  subtitle,
  mono,
  tooltip,
  children,
}: {
  title: string
  subtitle?: string
  mono?: boolean
  /** The raw ID, on hover only */
  tooltip?: string
  children: React.ReactNode
}) {
  return (
    <li className="flex items-center gap-3 px-3 py-2.5" title={tooltip}>
      <div className="min-w-0 flex-1">
        <p
          dir={mono ? 'ltr' : undefined}
          className={
            mono
              ? 'truncate text-start font-mono text-xs text-foreground'
              : 'truncate text-sm font-medium text-foreground'
          }
        >
          {title}
        </p>
        {subtitle && (
          <p className="truncate text-xs text-muted-foreground">{subtitle}</p>
        )}
      </div>
      {children}
    </li>
  )
}
