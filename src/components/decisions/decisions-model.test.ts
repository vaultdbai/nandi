import { describe, expect, it } from 'vitest'
import type { DecisionRecord } from '@/lib/bindings'
import {
  chatView,
  decisionTone,
  decisionWordKey,
  isFiltered,
  monthLabel,
  NO_FILTERS,
  splitDecision,
  toFilter,
} from './decisions-model'

const record = (overrides: Partial<DecisionRecord> = {}): DecisionRecord => ({
  at: 1_794_148_325_000,
  kind: 'approval',
  itemId: 'a1',
  itemLabel: 'run_report',
  decision: 'approved',
  decidedBy: 'Dev',
  reason: null,
  chatId: null,
  chatExists: false,
  ...overrides,
})

describe('decisions model', () => {
  it('sends no filter until one is set, then only the set fields', () => {
    expect(isFiltered(NO_FILTERS)).toBe(false)
    expect(toFilter(NO_FILTERS)).toBeNull()
    expect(isFiltered({ ...NO_FILTERS, search: '  ' })).toBe(false)
    expect(
      toFilter({ kind: 'approval', decision: '', search: ' board ' })
    ).toEqual({ kind: 'approval', decision: null, search: 'board' })
  })

  it('reads a decision as yes, no or neither, ignoring the choice detail', () => {
    expect(decisionTone('approved')).toBe('positive')
    expect(decisionTone('approved (once)')).toBe('positive')
    expect(decisionTone('Refused')).toBe('negative')
    expect(decisionTone('not now')).toBe('negative')
    expect(decisionTone('dismissed')).toBe('neutral')
    expect(decisionTone('something else')).toBe('neutral')
  })

  it('splits a decision into its known word and the rest', () => {
    expect(splitDecision('approved (once)')).toEqual({
      word: 'approved',
      rest: '(once)',
    })
    expect(splitDecision('not now')).toEqual({ word: 'not now', rest: '' })
    expect(splitDecision('reassigned to Sam')).toEqual({
      word: 'reassigned',
      rest: 'to Sam',
    })
    expect(splitDecision('approvedish')).toEqual({
      word: null,
      rest: 'approvedish',
    })
    expect(decisionWordKey('not now')).toBe('notNow')
  })

  it('opens a decision’s chat only while the chat exists', () => {
    expect(chatView(record())).toBeNull()
    expect(
      chatView(record({ chatId: 'inbox:approval:a1', chatExists: false }))
    ).toBeNull()
    expect(
      chatView(record({ chatId: 'inbox:approval:a1', chatExists: true }))
    ).toEqual({ kind: 'inbox-thread', conversationId: 'inbox:approval:a1' })
    expect(chatView(record({ chatId: 'intake:i1', chatExists: true }))).toEqual(
      { kind: 'intake-thread', conversationId: 'intake:i1' }
    )
    expect(chatView(record({ chatId: 'other:x', chatExists: true }))).toBeNull()
  })

  it('names a month in the reader’s language', () => {
    expect(monthLabel('2026-11', 'en')).toBe('November 2026')
    expect(monthLabel('2026-11', 'fr')).toBe('novembre 2026')
    expect(monthLabel('bad', 'en')).toBe('bad')
  })
})
