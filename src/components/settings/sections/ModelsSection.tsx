import { useTranslation } from 'react-i18next'
import { EnterpriseConfigSection } from '@/components/preferences/EnterpriseConfigSection'
import { HarnessSessionList } from '@/components/harness/HarnessSessionList'
import { ClaudeCodeToolsPanel } from '@/components/harness/ClaudeCodeToolsPanel'
import {
  SandboxBuildPanel,
  SandboxImagePanel,
} from '@/components/harness/SandboxImagePanel'
import {
  ModelChoiceFields,
  ProviderKeys,
} from '@/components/preferences/LlmKeysSection'
import {
  AdvancedBlock,
  SettingsGroup,
  SettingsSectionFrame,
} from '../SettingsFrame'
import { RuntimesPanel } from './RuntimesSection'

/**
 * Settings › Models & runtimes, one page for "what runs my agents":
 * Nandi's own loop (its model and the free tokens), the coding agents
 * that run in the sandbox (Claude Code, Codex, Gemini CLI), then the
 * provider keys both use, and team settings, sessions and the optional
 * local build of the sandbox images under Advanced. Nothing here has to be
 * done first: Nandi downloads what an agent needs on first use.
 */
export function ModelsSection() {
  const { t } = useTranslation()
  return (
    <SettingsSectionFrame
      title={t('models.title')}
      description={t('models.description')}
    >
      <SettingsGroup
        title={t('models.group.nandi')}
        description={t('models.group.nandiDescription')}
      >
        <ModelChoiceFields />
        <SandboxImagePanel image="runtime" />
      </SettingsGroup>
      <div data-settings-focus="runtimes" className="rounded-md">
        <SettingsGroup
          title={t('models.group.agents')}
          description={t('harness.subtitle')}
        >
          <SandboxImagePanel image="harness" />
          <ClaudeCodeToolsPanel />
          <RuntimesPanel />
        </SettingsGroup>
      </div>
      <SettingsGroup
        title={t('models.group.keys')}
        description={t('models.group.keysDescription')}
      >
        <ProviderKeys />
      </SettingsGroup>
      <AdvancedBlock>
        <SandboxBuildPanel />
        <EnterpriseConfigSection />
        <HarnessSessionList />
      </AdvancedBlock>
    </SettingsSectionFrame>
  )
}
