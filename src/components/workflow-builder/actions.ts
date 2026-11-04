import { toast } from 'sonner'
import i18n from '@/i18n/config'
import { client } from '@/lib/client'
import { logger } from '@/lib/logger'
import { queryClient } from '@/lib/query-client'

/**
 * Saves the workflow's YAML (validated by the core, written to its file).
 * Returns the error message, or null once saved.
 */
export async function saveWorkflowYaml(
  workflowId: string,
  yaml: string
): Promise<string | null> {
  const result = await client.saveWorkflowYaml(workflowId, yaml)
  if (result.status === 'error') {
    logger.warn('Workflow save failed', { workflowId, error: result.error })
    return result.error
  }
  toast.success(i18n.t('workflowBuilder.saved'))
  // The workflow, its graph (`['workflow', id, 'graph']`), and the list
  void queryClient.invalidateQueries({ queryKey: ['workflow', workflowId] })
  void queryClient.invalidateQueries({ queryKey: ['workflows'] })
  return null
}
