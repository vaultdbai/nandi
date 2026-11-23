/**
 * The agent's visible browser: its own profile, or My Nandi browser of one
 * browser type (the sign-ins the person made in Settings › Browser).
 * Choosing a shared profile is a grant the person gives this agent; its
 * allowed sites and per-call approvals still apply.
 */
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { useNandiBrowsers, useSetAgentBrowserProfile } from '@/hooks/usePilots'
import type { BrowserKind } from '@/lib/bindings'
import {
  BROWSER_LABELS,
  fromChoice,
  profileChoices,
  toChoice,
} from '@/lib/browser-profile'

export function BrowserProfileSelect({
  agentId,
  pilotName,
  current,
}: {
  agentId: string
  pilotName: string
  current: BrowserKind | null
}) {
  const { t } = useTranslation()
  const browsers = useNandiBrowsers()
  const set = useSetAgentBrowserProfile(agentId)
  const installed = (browsers.data ?? []).map(b => b.browser)
  const choices = profileChoices(installed, current)
  return (
    <NativeSelect
      aria-label={t('editor.permissions.profileLabel', { name: pilotName })}
      value={toChoice(current)}
      disabled={set.isPending}
      onChange={event =>
        set.mutate(fromChoice(event.target.value), {
          onError: error =>
            toast.error(t('editor.permissions.profileFailed'), {
              description: String(
                error instanceof Error ? error.message : error
              ),
            }),
        })
      }
    >
      {choices.map(choice => (
        <NativeSelectOption key={choice} value={choice}>
          {choice === 'own'
            ? t('editor.permissions.profileOwn')
            : t('editor.permissions.profileShared', {
                browser: BROWSER_LABELS[choice],
              })}
        </NativeSelectOption>
      ))}
    </NativeSelect>
  )
}
