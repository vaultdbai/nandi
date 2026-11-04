/**
 * Which Settings sections need attention: no model to call, the default
 * runtime not ready, a source failing, the backup unreachable. The Settings
 * rail and the sidebar both show these as dots.
 */
import { runtimeStatus } from '@/components/harness/runtime-options'
import { useCapabilityRegistries } from '@/hooks/useCapabilityRegistries'
import { useHarnesses } from '@/hooks/useHarnesses'
import { useDefaultModel } from '@/hooks/useLlmKeyStatus'
import { useRegistries } from '@/hooks/useMarketplace'
import { useRemoteStatus } from '@/hooks/useRemote'
import type { SettingsSection } from '@/lib/settings-nav'

export function useSettingsAttention(): Partial<
  Record<SettingsSection, boolean>
> {
  const model = useDefaultModel()
  const harnesses = useHarnesses()
  const marketplace = useRegistries()
  const capabilities = useCapabilityRegistries()
  const remote = useRemoteStatus()
  const defaultRuntime = harnesses.data?.find(h => h.isDefault)
  return {
    // Models & runtimes: no model to call, or the default runtime not ready
    models:
      model.data?.effectiveModel === null ||
      (defaultRuntime !== undefined &&
        runtimeStatus(defaultRuntime) !== 'ready'),
    sources:
      (marketplace.data ?? []).some(r => r.warning) ||
      (capabilities.data ?? []).some(r => r.lastError),
    backup:
      remote.data?.configured === true &&
      (!remote.data.reachable || remote.data.conflicts),
  }
}
