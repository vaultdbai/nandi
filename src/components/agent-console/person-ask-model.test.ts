import { describe, expect, it } from 'vitest'
import { answerText, askStage } from './person-ask-model'

describe('askStage', () => {
  it('reads sent, seen, answered or not from the task', () => {
    expect(askStage('submitted')).toBe('sent')
    expect(askStage('delivered')).toBe('sent')
    expect(askStage('acknowledged')).toBe('seen')
    expect(askStage('working')).toBe('seen')
    expect(askStage('completed')).toBe('answered')
    expect(askStage('rejected')).toBe('notAnswered')
    expect(askStage('timedOut')).toBe('notAnswered')
  })
})

describe('answerText', () => {
  it('is the answer in words', () => {
    expect(answerText({ text: 'Looks right.' })).toBe('Looks right.')
    expect(answerText({ output: { answer: 'Kim owns it' } })).toBe(
      'Kim owns it'
    )
    expect(answerText('Done')).toBe('Done')
    expect(answerText(null)).toBeNull()
    expect(answerText({ n: 1 })).toBe('{"n":1}')
  })
})
