import { describe, expect, it } from 'vitest'
import type { UiMessage } from '@/types/agent'
import { foldSteps } from './steps-model'

const said = (id: string): UiMessage => ({
  kind: 'agent',
  id,
  runId: 'r1',
  createdAt: 0,
  content: id,
})

const call = (id: string, tool = 'fs_read'): UiMessage => ({
  kind: 'toolCall',
  id,
  runId: 'r1',
  createdAt: 0,
  tool,
  args: {},
  status: 'done',
  result: null,
  isError: false,
})

describe('foldSteps', () => {
  it('folds each stretch of tool calls, keeping words and cards apart', () => {
    const items = foldSteps([
      said('a'),
      call('t1'),
      call('t2'),
      call('plan', 'plan_set'),
      call('t3'),
      said('b'),
    ])
    expect(
      items.map(item =>
        item.kind === 'steps'
          ? item.messages.map(m => m.id).join('+')
          : item.message.id
      )
    ).toEqual(['a', 't1+t2', 'plan', 't3', 'b'])
    expect(items[1]).toMatchObject({ kind: 'steps', id: 'steps:t1' })
  })

  it('leaves a chat with no steps as it is', () => {
    expect(foldSteps([said('a')])).toEqual([
      { kind: 'message', message: said('a') },
    ])
  })
})
