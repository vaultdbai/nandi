import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Label } from '@/components/ui/label'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { Spinner } from '@/components/ui/spinner'
import { client } from '@/lib/client'
import type { RelayUrlSource } from '@/lib/bindings'
import {
  isRelayUrl,
  relayHost,
  relayNeedsSave,
  relayUrlFor,
  skipTurnsRelayOff,
  type RelayChoice,
  type RelayDraft,
} from './identity-model'
import { plainError } from '@/lib/toast-error'
import {
  JoinRelayFields,
  JoinResult,
} from '@/components/network/JoinRelayFields'
import { joinMessage, type JoinMessage } from '@/components/network/join-model'
import { PublicRelayJoin } from '@/components/network/PublicRelayJoin'

export interface Step3NetworkProps {
  value: RelayDraft
  onChange: (value: RelayDraft) => void
  onNext: () => void
  onBack: () => void
  /** The relay URL already configured (`null`: none) */
  configuredUrl?: string | null
  /** Where `configuredUrl` comes from */
  configuredSource?: RelayUrlSource | null
  /** The relay this build comes with; `null`: no Nandi network to offer */
  builtInUrl?: string | null
  /** The public relay the app offers; `null`: none to offer */
  publicUrl?: string | null
  /** The configured relay is still being read; Continue waits for it */
  checking?: boolean
}

/**
 * Onboarding step 3 (optional): the network to join. Nothing joins until
 * the user says so. "Join the public relay" joins with the connected
 * email and connects once joined; it never holds Continue up, and an
 * unreachable relay just says to join later. For a company relay, "Join"
 * or Continue joins it, enrolling with the join code where the relay needs
 * one (the build's own code for the build's relay). A failed join says
 * why, and the next Continue goes on without a network. "Let others send
 * requests" is off until ticked. The relay already in use is never joined
 * again.
 */
export function Step3Network({
  value,
  onChange,
  onNext,
  onBack,
  configuredUrl = null,
  configuredSource = null,
  builtInUrl = null,
  publicUrl = null,
  checking = false,
}: Step3NetworkProps) {
  const { t } = useTranslation()
  const [joining, setJoining] = useState(false)
  const [message, setMessage] = useState<JoinMessage | null>(null)
  // The relay joined here: the core already uses it
  const [joinedUrl, setJoinedUrl] = useState<string | null>(null)
  const currentUrl = joinedUrl ?? configuredUrl

  const url = relayUrlFor(value, builtInUrl)
  const urlInvalid = value.choice === 'custom' && !isRelayUrl(value.customUrl)
  const failed = message !== null && !message.ok

  const localFailure = (error: string): JoinMessage => ({
    key: 'onboarding.relay.saveFailed',
    params: { error: plainError(error) },
    detail: null,
    ok: false,
  })

  /** Joins the chosen network; whether it is in use now */
  const join = async (draft: RelayDraft): Promise<boolean> => {
    const target = relayUrlFor(draft, builtInUrl)
    if (!target || (draft.choice === 'custom' && !isRelayUrl(target)))
      return false
    if (!relayNeedsSave(draft, currentUrl, builtInUrl)) return true
    setJoining(true)
    const result = await client.joinRelay(
      target,
      draft.choice === 'custom' ? draft.joinCode?.trim() || null : null
    )
    setJoining(false)
    if (result.status === 'error') {
      setMessage(localFailure(result.error))
      return false
    }
    setMessage(joinMessage(result.data, target))
    if (result.data.kind !== 'joined') return false
    setJoinedUrl(target)
    // Never kept or shown once used
    onChange({ ...draft, joinCode: '' })
    return true
  }

  const edit = (next: RelayDraft) => {
    onChange(next)
    setMessage(null)
  }

  const next = async () => {
    if (skipTurnsRelayOff(value, configuredSource, joinedUrl !== null)) {
      // "Not now" after joining here: leave the network again
      setJoining(true)
      const off = await client.setRelayUrl('')
      setJoining(false)
      if (off.status === 'error' && !failed) {
        setMessage(localFailure(off.error))
        return
      }
      setJoinedUrl(null)
      onNext()
      return
    }
    if (url === null || failed) {
      onNext()
      return
    }
    if (urlInvalid) return
    if (await join(value)) onNext()
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="space-y-1">
        <h2 className="text-lg font-semibold">{t('onboarding.relay.title')}</h2>
        <p className="text-sm text-muted-foreground">
          {t('onboarding.relay.description')}
        </p>
      </div>

      <RadioGroup
        value={value.choice}
        onValueChange={choice =>
          edit({ ...value, choice: choice as RelayChoice })
        }
        aria-label={t('onboarding.relay.title')}
      >
        <ChoiceRow
          value="skip"
          label={t('onboarding.relay.skip')}
          description={t('onboarding.relay.skipDescription')}
        />
        {publicUrl && (
          <ChoiceRow
            value="public"
            label={t('onboarding.relay.public')}
            description={t('onboarding.relay.publicDescription')}
          />
        )}
        {builtInUrl && (
          <ChoiceRow
            value="builtIn"
            label={t('onboarding.relay.builtIn')}
            description={t('onboarding.relay.builtInDescription', {
              host: relayHost(builtInUrl),
            })}
          />
        )}
        <ChoiceRow
          value="custom"
          label={t('onboarding.relay.custom')}
          description={t('onboarding.relay.customDescription')}
        />
      </RadioGroup>

      {configuredSource === 'env' && configuredUrl && (
        <p
          className="text-xs text-muted-foreground"
          data-testid="relay-env-note"
        >
          {t('onboarding.relay.fromEnv')}
        </p>
      )}

      {value.choice === 'public' && (
        <div className="grid gap-2" data-testid="onboarding-public-relay">
          {value.signedInRelay ? (
            <p role="status" className="text-sm text-success">
              {t('onboarding.relay.publicConnected')}
            </p>
          ) : (
            <PublicRelayJoin
              onLinked={relay => {
                setJoinedUrl(relay)
                onChange({ ...value, signedInRelay: relay })
              }}
            />
          )}
        </div>
      )}

      {value.choice === 'custom' && (
        <div className="grid gap-2">
          <JoinRelayFields
            idPrefix="onboarding-relay"
            urlLabel={t('onboarding.relay.urlLabel')}
            value={{ url: value.customUrl, joinCode: value.joinCode ?? '' }}
            onChange={fields =>
              edit({
                ...value,
                customUrl: fields.url,
                joinCode: fields.joinCode,
              })
            }
            invalid={Boolean(value.customUrl) && urlInvalid}
            onUrlKeyDown={event => {
              if (event.key === 'Enter') void join(value)
            }}
          />
          {value.customUrl && urlInvalid && (
            <p className="text-sm text-destructive">
              {t('onboarding.relay.urlInvalid')}
            </p>
          )}
          <div>
            <Button
              size="sm"
              variant="outline"
              disabled={joining || urlInvalid}
              onClick={() => void join(value)}
            >
              {t('relay.join.join')}
            </Button>
          </div>
        </div>
      )}

      <JoinResult pending={joining} message={message} />

      {url !== null && (
        <div className="flex flex-col gap-2 rounded-md border p-3">
          <div className="flex items-start gap-3">
            <Checkbox
              id="onboarding-relay-reachable"
              checked={value.reachable === true}
              onCheckedChange={checked =>
                onChange({ ...value, reachable: checked === true })
              }
              className="mt-0.5"
            />
            <div className="space-y-0.5">
              <Label htmlFor="onboarding-relay-reachable">
                {t('onboarding.relay.reachable')}
              </Label>
              <p className="text-xs text-muted-foreground">
                {t('onboarding.relay.reachableHint')}
              </p>
            </div>
          </div>
          {value.reachable === true && (
            <div
              className="space-y-1 ps-7 text-xs text-muted-foreground"
              data-testid="reachable-note"
            >
              <p>{t('onboarding.relay.reachableNote.intro')}</p>
              <ul className="list-disc space-y-0.5 ps-4">
                <li>{t('onboarding.relay.reachableNote.accept')}</li>
                <li>{t('onboarding.relay.reachableNote.trust')}</li>
              </ul>
              <p>{t('onboarding.relay.reachableNote.later')}</p>
            </div>
          )}
        </div>
      )}

      <div className="flex justify-between gap-2">
        <Button variant="ghost" onClick={onBack}>
          {t('onboarding.back')}
        </Button>
        <Button
          disabled={checking || joining || (url !== null && urlInvalid)}
          onClick={() => void next()}
        >
          {(joining || checking) && <Spinner />}
          {failed ? t('onboarding.continueAnyway') : t('onboarding.continue')}
        </Button>
      </div>
    </div>
  )
}

function ChoiceRow({
  value,
  label,
  description,
}: {
  value: RelayChoice
  label: string
  description: string
}) {
  const id = `onboarding-relay-${value}`
  return (
    <div className="flex items-start gap-3 rounded-md border p-3">
      <RadioGroupItem id={id} value={value} className="mt-0.5" />
      <div className="space-y-0.5">
        <Label htmlFor={id}>{label}</Label>
        <p className="text-xs text-muted-foreground">{description}</p>
      </div>
    </div>
  )
}
