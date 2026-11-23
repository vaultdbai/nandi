import { describe, expect, it } from 'vitest'
import { PLANS, planView, type PlanId } from './plans-model'
import en from '../../../locales/en.json'

const strings = en as Record<string, string>

function must(id: PlanId, trialOffered: boolean) {
  const view = planView(id, trialOffered)
  if (!view) throw new Error(`no plan ${id}`)
  return view
}

describe('plans-model', () => {
  it('lists each plan once, in order', () => {
    expect(PLANS.map(p => p.id)).toEqual([
      'claude',
      'cloud',
      'chatgpt',
      'gemini',
      'copilot',
      'nothing',
    ])
  })

  it('has an English text for every name, step and action', () => {
    for (const { id } of PLANS) {
      for (const trial of [true, false]) {
        const view = must(id, trial)
        expect(strings[view.nameKey]).toBeTruthy()
        view.stepKeys.forEach(k => expect(strings[k]).toBeTruthy())
        view.actions.forEach(a => expect(strings[a.labelKey]).toBeTruthy())
      }
    }
  })

  it('keeps every plan to three steps or fewer', () => {
    for (const { id } of PLANS) {
      expect(must(id, true).stepKeys.length).toBeLessThanOrEqual(3)
    }
  })

  it('shows the free tokens step only when the build offers them', () => {
    expect(must('nothing', true).stepKeys).toHaveLength(2)
    expect(must('nothing', false).stepKeys).toEqual([
      'onboarding.plans.nothing.step2',
    ])
    expect(must('gemini', false).stepKeys).not.toContain(
      'onboarding.plans.gemini.step2'
    )
  })

  it('points ChatGPT at an OpenAI key, not the subscription', () => {
    const view = must('chatgpt', false)
    expect(view.actions).toContainEqual({
      kind: 'link',
      url: 'https://platform.openai.com/api-keys',
      labelKey: 'onboarding.plans.action.openAiKeys',
    })
    expect(view.actions.some(a => a.kind === 'provider')).toBe(true)
  })

  it('never offers a Gemini key form', () => {
    const view = must('gemini', true)
    expect(view.actions.every(a => a.kind === 'ollama')).toBe(true)
  })

  it('opens only https links', () => {
    for (const plan of PLANS) {
      for (const a of plan.actions) {
        if (a.kind === 'link') expect(a.url.startsWith('https://')).toBe(true)
      }
    }
  })
})
