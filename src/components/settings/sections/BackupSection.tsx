import { useTranslation } from 'react-i18next'
import { LocalCopyPanel } from '../LocalCopyPanel'
import { RemoteSection } from '../RemoteSection'
import { AdvancedBlock, SettingsSectionFrame } from '../SettingsFrame'

/** Settings › Backup: the git remote, and this computer's copy */
export function BackupSection() {
  const { t } = useTranslation()
  return (
    <SettingsSectionFrame
      title={t('settings.section.backup')}
      description={t('settings.backup.description')}
    >
      <RemoteSection />
      <AdvancedBlock label={t('settings.backup.localCopy')}>
        <LocalCopyPanel />
      </AdvancedBlock>
    </SettingsSectionFrame>
  )
}
