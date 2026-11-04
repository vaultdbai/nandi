import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { RelayConnectNotice } from '@/components/network/RelayConnectNotice'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Spinner } from '@/components/ui/spinner'
import { Textarea } from '@/components/ui/textarea'
import {
  autoProvider,
  parseCallInput,
  providerLabel,
  useCallSharedAgent,
  useCapabilityProviders,
} from '@/hooks/useSharedAgents'
import { isRelayUnavailable } from '@/lib/friendly-error'

/** What the dialog sends: a title, the capabilities to choose from, and
 *  who it goes to first (`null`: let Nandi pick) */
export interface CallRequest {
  title: string
  capabilities: string[]
  target: string | null
}

/** `select` value for "let Nandi pick" (card IDs always contain a slash) */
const AUTO = 'auto'

/**
 * Quick delegation: pick a capability, who it goes to (Auto shows who it
 * would pick now), and the input.
 */
export function CallDialog({
  request,
  onClose,
}: {
  request: CallRequest | null
  onClose: () => void
}) {
  return (
    <Dialog open={request !== null} onOpenChange={open => !open && onClose()}>
      <DialogContent>
        {request && (
          <CallForm
            key={`${request.title}:${request.target ?? AUTO}`}
            request={request}
            onDone={onClose}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}

function CallForm({
  request,
  onDone,
}: {
  request: CallRequest
  onDone: () => void
}) {
  const { t } = useTranslation()
  const call = useCallSharedAgent()
  const [capability, setCapability] = useState(request.capabilities[0] ?? '')
  const [target, setTarget] = useState(request.target ?? AUTO)
  const [input, setInput] = useState('')
  const [relayRefused, setRelayRefused] = useState(false)
  const providers = useCapabilityProviders(capability)

  const entry = providers.data?.[0]
  const auto = autoProvider(entry)
  // Anyone who can take it now, plus the one the dialog was opened for
  const choices = (entry?.providers ?? []).filter(
    provider => provider.rank !== null || provider.card.id === request.target
  )

  const submit = () => {
    call.mutate(
      {
        cardId: target === AUTO ? null : target,
        capability,
        input: parseCallInput(input),
      },
      {
        onSuccess: taskId => {
          toast.success(t('shared.callStarted', { taskId }))
          onDone()
        },
        onError: error => {
          if (isRelayUnavailable(error)) setRelayRefused(true)
          else toast.error(t('shared.callFailed', { error: error.message }))
        },
      }
    )
  }

  return (
    <form
      className="flex flex-col gap-4"
      onSubmit={event => {
        event.preventDefault()
        submit()
      }}
    >
      <DialogHeader>
        <DialogTitle>{request.title}</DialogTitle>
        <DialogDescription>{t('shared.callDescription')}</DialogDescription>
      </DialogHeader>
      <div className="flex flex-col gap-2">
        <Label htmlFor="shared-call-capability">{t('shared.capability')}</Label>
        <NativeSelect
          id="shared-call-capability"
          value={capability}
          onChange={event => setCapability(event.target.value)}
        >
          {request.capabilities.map(name => (
            <NativeSelectOption key={name} value={name}>
              {name}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </div>
      <div className="flex flex-col gap-2">
        <Label htmlFor="shared-call-target">{t('shared.sendTo')}</Label>
        <NativeSelect
          id="shared-call-target"
          value={target}
          onChange={event => setTarget(event.target.value)}
        >
          <NativeSelectOption value={AUTO}>
            {providers.isPending
              ? t('shared.autoLoading')
              : auto
                ? t('shared.autoWillUse', { name: providerLabel(auto) })
                : t('shared.autoNoOne')}
          </NativeSelectOption>
          {choices.map(provider => (
            <NativeSelectOption key={provider.card.id} value={provider.card.id}>
              {providerLabel(provider)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
        <p className="text-xs text-muted-foreground">
          {target === AUTO ? t('shared.autoHint') : t('shared.pickedHint')}
        </p>
      </div>
      <div className="flex flex-col gap-2">
        <Label htmlFor="shared-call-input">{t('shared.input')}</Label>
        <Textarea
          id="shared-call-input"
          value={input}
          onChange={event => setInput(event.target.value)}
          placeholder={t('shared.inputPlaceholder')}
          rows={5}
        />
      </div>
      {relayRefused && <RelayConnectNotice variant="inline" />}
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onDone}>
          {t('shared.cancel')}
        </Button>
        <Button type="submit" disabled={!capability || call.isPending}>
          {call.isPending && <Spinner />}
          {t('shared.send')}
        </Button>
      </DialogFooter>
    </form>
  )
}
