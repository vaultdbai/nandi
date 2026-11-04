import { useTranslation } from 'react-i18next'
import { StarterPackPicker } from '@/components/marketplace/StarterPackPicker'
import { ScrollArea } from '@/components/ui/scroll-area'
import { useNavigationStore } from '@/store/navigation-store'
import { ViewHeader } from './ViewHeader'

/**
 * Pick curated packs from the configured registries and install them in
 * one go (also onboarding's last step). Lands on the Dashboard, which
 * welcomes the new agents with something to try.
 */
export function StarterPacksView() {
  const { t } = useTranslation()

  const toHome = (packIds: string[]) =>
    useNavigationStore
      .getState()
      .navigate(
        packIds.length > 0
          ? { kind: 'home', welcome: { packIds } }
          : { kind: 'home' }
      )

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader
        title={t('starterPacks.title')}
        subtitle={t('starterPacks.subtitle')}
      />
      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto w-full max-w-7xl p-6">
          <StarterPackPicker onDone={toHome} onSkip={() => toHome([])} />
        </div>
      </ScrollArea>
    </div>
  )
}
