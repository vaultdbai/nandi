import type { KeyedProvider } from '@/lib/tauri-bindings'

/** The plans a person can say they already pay for */
export type PlanId =
  'claude' | 'cloud' | 'chatgpt' | 'gemini' | 'copilot' | 'nothing'

/** What a plan's button does: open a page, or open a form already on the step */
export type PlanAction =
  | { kind: 'link'; url: string; labelKey: string }
  | { kind: 'provider'; provider: KeyedProvider; labelKey: string }
  | { kind: 'ollama'; labelKey: string }

interface PlanStep {
  key: string
  /** Shown only when the build offers the free tokens */
  needsTrial: boolean
}

interface Plan {
  id: PlanId
  steps: PlanStep[]
  actions: PlanAction[]
}

/** What the chooser shows for one plan, ready to render */
export interface PlanView {
  id: PlanId
  nameKey: string
  stepKeys: string[]
  actions: PlanAction[]
}

const step = (id: PlanId, n: number, needsTrial = false): PlanStep => ({
  key: `onboarding.plans.${id}.step${n}`,
  needsTrial,
})

/**
 * The plans, in the order shown. Each says only what the plan really
 * allows: a chat subscription is not an API key, so those plans say so and
 * point at what does work. The texts are in `locales/*.json` under
 * `onboarding.plans.<id>`.
 */
export const PLANS: readonly Plan[] = [
  {
    id: 'claude',
    steps: [step('claude', 1), step('claude', 2)],
    actions: [
      {
        kind: 'link',
        url: 'https://docs.claude.com/en/docs/claude-code/setup',
        labelKey: 'onboarding.plans.action.claudeCode',
      },
    ],
  },
  {
    id: 'cloud',
    steps: [step('cloud', 1), step('cloud', 2)],
    actions: [],
  },
  {
    id: 'chatgpt',
    steps: [step('chatgpt', 1), step('chatgpt', 2), step('chatgpt', 3)],
    actions: [
      {
        kind: 'link',
        url: 'https://platform.openai.com/api-keys',
        labelKey: 'onboarding.plans.action.openAiKeys',
      },
      {
        kind: 'provider',
        provider: 'openAi',
        labelKey: 'onboarding.plans.action.openAiForm',
      },
    ],
  },
  {
    id: 'gemini',
    steps: [step('gemini', 1), step('gemini', 2, true), step('gemini', 3)],
    actions: [
      { kind: 'ollama', labelKey: 'onboarding.plans.action.ollamaForm' },
    ],
  },
  {
    id: 'copilot',
    steps: [step('copilot', 1), step('copilot', 2)],
    actions: [],
  },
  {
    id: 'nothing',
    steps: [step('nothing', 1, true), step('nothing', 2)],
    actions: [
      {
        kind: 'link',
        url: 'https://ollama.com/download',
        labelKey: 'onboarding.plans.action.ollamaDownload',
      },
      { kind: 'ollama', labelKey: 'onboarding.plans.action.ollamaForm' },
    ],
  },
]

/** The plan's texts and actions for this build, or `null` for an unknown id */
export function planView(id: PlanId, trialOffered: boolean): PlanView | null {
  const plan = PLANS.find(p => p.id === id)
  if (!plan) return null
  return {
    id,
    nameKey: `onboarding.plans.${id}.name`,
    stepKeys: plan.steps
      .filter(s => trialOffered || !s.needsTrial)
      .map(s => s.key),
    actions: plan.actions,
  }
}
