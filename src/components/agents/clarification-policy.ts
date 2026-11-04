/** The clarification policy model the editor and the status views share */
import type { ClarificationPolicy } from '@/lib/bindings'

/** `ClarificationPolicy::default()` */
export const DEFAULT_CLARIFICATION_POLICY: ClarificationPolicy = {
  allow: true,
  maxRounds: 2,
  autoTopics: [],
  escalateTopics: [],
  timeoutSeconds: 300,
}

export const MAX_CLARIFICATION_ROUNDS = 5

/** One line for the policy, e.g. "Up to 2 rounds, 300s each" */
export function summarizePolicy(
  policy: ClarificationPolicy,
  t: (key: string, options?: Record<string, unknown>) => string
): string {
  if (!policy.allow) return t('tasks.policy.summaryOff')
  return t('tasks.policy.summary', {
    count: policy.maxRounds,
    seconds: policy.timeoutSeconds,
  })
}
