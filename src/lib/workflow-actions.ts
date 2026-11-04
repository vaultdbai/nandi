/**
 * Workflow user actions. Run progress arrives back as `workflow://*`
 * events, which refresh the workflow queries (see `agent-sync`).
 */

import { toast } from 'sonner'
import i18n from '@/i18n/config'
import { client } from '@/lib/client'
import { logger } from '@/lib/logger'
import { queryClient } from '@/lib/query-client'
import type { TemplateWorkflowInput } from '@/lib/bindings'
import type { ApprovalDecision } from '@/lib/tauri-bindings'
import { useNavigationStore } from '@/store/navigation-store'

/** Starts a run; returns its ID, or null (with a toast) on failure. */
export async function runWorkflow(workflowId: string): Promise<string | null> {
  const result = await client.runWorkflow(workflowId)
  void queryClient.invalidateQueries({ queryKey: ['workflow', workflowId] })
  if (result.status === 'error') {
    logger.warn('Workflow run failed to start', {
      workflowId,
      error: result.error,
    })
    toast.error(result.error)
    return null
  }
  toast.success(i18n.t('workflows.runStarted'))
  return result.data
}

/** Imports a YAML file into the agents repo and opens it. */
export async function importWorkflowFile(file: File): Promise<void> {
  const yaml = await file.text()
  const result = await client.createWorkflow({ yaml })
  if (result.status === 'error') {
    logger.warn('Workflow import failed', { name: file.name })
    toast.error(result.error)
    return
  }
  toast.success(i18n.t('workflows.imported', { name: result.data.name }))
  void queryClient.invalidateQueries({ queryKey: ['workflows'] })
  useNavigationStore
    .getState()
    .navigate({ kind: 'workflow', workflowId: result.data.id })
}

/** Builds a template with the chosen agents, saves it, and opens its Graph */
export async function createWorkflowFromTemplate(
  input: TemplateWorkflowInput
): Promise<boolean> {
  const result = await client.createWorkflowFromTemplate(input)
  if (result.status === 'error') {
    logger.warn('Workflow from template failed', {
      templateId: input.templateId,
    })
    toast.error(result.error)
    return false
  }
  toast.success(i18n.t('workflows.imported', { name: result.data.name }))
  void queryClient.invalidateQueries({ queryKey: ['workflows'] })
  useNavigationStore.getState().navigate({
    kind: 'workflow',
    workflowId: result.data.id,
    tab: 'graph',
  })
  return true
}

export async function approveWorkflowStep(
  workflowId: string,
  runId: string,
  stepId: string,
  decision: ApprovalDecision
): Promise<void> {
  const result = await client.approveWorkflowStep(runId, stepId, decision)
  if (result.status === 'error') toast.error(result.error)
  void queryClient.invalidateQueries({ queryKey: ['workflow', workflowId] })
}
