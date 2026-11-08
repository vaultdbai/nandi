import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'
import { AppearancePane } from '@/components/preferences/panes/AppearancePane'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { AuditLogView } from '@/components/views/AuditLogView'
import {
  DEFAULT_SETTINGS_SECTION,
  isSettingsSection,
  SETTINGS_SECTIONS,
  type SettingsSection,
} from '@/lib/settings-nav'
import { cn } from '@/lib/utils'
import { useNavigationStore, type View } from '@/store/navigation-store'
import { useUIStore } from '@/store/ui-store'
import { useSettingsAttention } from '@/hooks/useSettingsAttention'
import { useSettingsFocus } from '@/hooks/useSettingsFocus'
import { SettingsSectionFrame } from './SettingsFrame'
import { AccountSection } from './sections/AccountSection'
import { BackupSection } from './sections/BackupSection'
import { CloudCredentialsSection } from './sections/CloudCredentialsSection'
import { AutoApproveSection } from './sections/AutoApproveSection'
import { ConnectorAccessSection } from './sections/ConnectorAccessSection'
import { EmailSection } from './sections/EmailSection'
import { InternetAccessSection } from './sections/InternetAccessSection'
import { ModelsSection } from './sections/ModelsSection'
import { SourcesSection } from './sections/SourcesSection'
import { WorkspaceSection } from './sections/WorkspaceSection'

/**
 * Settings: one full page with a section rail (a select below 900 px).
 * Opens on the view's section, else the last one used this session, else
 * Models.
 */
export function SettingsView({
  view,
}: {
  view: Extract<View, { kind: 'settings' }>
}) {
  const { t } = useTranslation()
  const last = useUIStore(state => state.settingsSection)
  const section: SettingsSection = isSettingsSection(view.section)
    ? view.section
    : (last ?? DEFAULT_SETTINGS_SECTION)
  const attention = useSettingsAttention()

  useEffect(() => {
    useUIStore.getState().setSettingsSection(section)
  }, [section])
  useSettingsFocus(view.focus)

  const select = (next: SettingsSection) =>
    useNavigationStore.getState().navigate({ kind: 'settings', section: next })

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 items-center gap-3 border-b px-6 py-3">
        <h1 className="text-base font-semibold">{t('nav.settings')}</h1>
        <NativeSelect
          className="min-[900px]:hidden"
          aria-label={t('settings.sectionLabel')}
          value={section}
          onChange={event => {
            if (isSettingsSection(event.target.value))
              select(event.target.value)
          }}
        >
          {SETTINGS_SECTIONS.map(id => (
            <NativeSelectOption key={id} value={id}>
              {t(`settings.section.${id}`)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </header>
      <div className="flex min-h-0 flex-1">
        <nav
          aria-label={t('settings.sectionLabel')}
          className="hidden w-[200px] shrink-0 border-e p-2 min-[900px]:block"
        >
          <ul className="flex flex-col gap-0.5">
            {SETTINGS_SECTIONS.map(id => (
              <li key={id}>
                <button
                  type="button"
                  aria-current={id === section ? 'page' : undefined}
                  className={cn(
                    'flex w-full items-center gap-2 rounded-md px-3 py-1.5 text-start text-sm hover:bg-accent',
                    id === section && 'bg-accent font-medium'
                  )}
                  onClick={() => select(id)}
                >
                  <span className="flex-1">{t(`settings.section.${id}`)}</span>
                  {attention[id] && (
                    <span
                      className="size-2 rounded-full bg-warning"
                      role="img"
                      aria-label={t('settings.needsAttention')}
                    />
                  )}
                </button>
              </li>
            ))}
          </ul>
        </nav>
        <main className="min-h-0 min-w-0 flex-1" data-section={section}>
          {section === 'models' && <ModelsSection />}
          {section === 'account' && <AccountSection />}
          {section === 'email' && <EmailSection />}
          {section === 'workspace' && <WorkspaceSection />}
          {section === 'access' && <ConnectorAccessSection />}
          {section === 'autoApprove' && <AutoApproveSection />}
          {section === 'cloud' && <CloudCredentialsSection />}
          {section === 'internet' && (
            <InternetAccessSection agentId={view.agentId} />
          )}
          {section === 'backup' && <BackupSection />}
          {section === 'sources' && <SourcesSection />}
          {section === 'logs' && <AuditLogView tab={view.logTab} />}
          {section === 'appearance' && <AppearanceSection />}
        </main>
      </div>
    </div>
  )
}

function AppearanceSection() {
  const { t } = useTranslation()
  return (
    <SettingsSectionFrame
      title={t('settings.section.appearance')}
      description={t('settings.appearance.description')}
    >
      <AppearancePane />
    </SettingsSectionFrame>
  )
}
