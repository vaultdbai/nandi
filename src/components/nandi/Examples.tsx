import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Plug } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { useAgents } from '@/hooks/useAgents'
import { useEmailStatus } from '@/hooks/useEmail'
import { useOwnDevices } from '@/hooks/useOwnDevices'
import { useRelayConnected } from '@/hooks/useRelay'
import { useWorkflows } from '@/hooks/useSnapshots'
import { useTriggers } from '@/hooks/useTriggers'
import { ownAgents } from '@/lib/agent-identity'
import { openSettings } from '@/lib/settings-nav'
import { useExamplesStore } from '@/store/examples-store'
import { useNavigationStore } from '@/store/navigation-store'
import {
  pickExamples,
  shownIds,
  type ExampleKind,
  type ExampleNeed,
} from './examples-model'

/** What's connected now, read from what the app already knows */
function useConnected(): Set<ExampleNeed> {
  const mail = (useEmailStatus().data?.mailboxes.length ?? 0) > 0
  const relay = useRelayConnected() === true
  const phone = (useOwnDevices().data?.devices ?? []).some(
    device => device.status === 'active'
  )
  const connected = new Set<ExampleNeed>()
  if (mail) connected.add('mail')
  if (relay) connected.add('relay')
  if (phone) connected.add('phone')
  return connected
}

/** Kinds already used: tapped examples, and what exists already */
function useUsed(): Set<ExampleKind> {
  const tapped = useExamplesStore(state => state.tapped)
  const workflows = useWorkflows().data?.length ?? 0
  const triggers = useTriggers().data?.length ?? 0
  const agents = ownAgents(useAgents().data ?? []).length
  const used = new Set<ExampleKind>(tapped)
  if (workflows + triggers > 0) used.add('automations')
  if (agents > 0) used.add('agents')
  return used
}

/** Where each connect step goes */
function connect(need: ExampleNeed) {
  switch (need) {
    case 'mail':
      return openSettings('email')
    case 'phone':
      return openSettings('devices')
    case 'relay':
      return useNavigationStore
        .getState()
        .navigate({ kind: 'network', tab: 'connection' })
  }
}

/**
 * Examples that teach, above the box: a new set each time this opens,
 * what the person hasn't tried first, and the step that connects what an
 * example would need. Tapping an ask sends it.
 */
export function Examples({ onPick }: { onPick: (text: string) => void }) {
  const { t } = useTranslation()
  // A new set each time it opens, never the last one again
  const [seed] = useState(() => Math.floor(Math.random() * 2 ** 31))
  const [previous] = useState(() => useExamplesStore.getState().lastShown)
  const chips = pickExamples(seed, {
    connected: useConnected(),
    used: useUsed(),
    previous,
  })
  const ids = shownIds(chips).join(',')
  useEffect(() => {
    useExamplesStore.getState().showed(ids ? ids.split(',') : [])
  }, [ids])

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-wrap items-center gap-2">
      <span className="text-xs text-muted-foreground">
        {t('examples.label')}
      </span>
      {chips.map(chip =>
        chip.kind === 'ask' ? (
          <Button
            key={chip.example.id}
            variant="outline"
            size="sm"
            className="h-auto py-1 text-xs whitespace-normal"
            onClick={() => {
              useExamplesStore.getState().tap(chip.example.kind)
              onPick(t(`examples.${chip.example.id}`))
            }}
          >
            {t(`examples.${chip.example.id}`)}
          </Button>
        ) : (
          <Button
            key={`connect:${chip.need}`}
            variant="outline"
            size="sm"
            className="h-auto border-dashed py-1 text-xs whitespace-normal"
            onClick={() => connect(chip.need)}
          >
            <Plug aria-hidden />
            {t(`examples.connect.${chip.need}`)}
          </Button>
        )
      )}
    </div>
  )
}
