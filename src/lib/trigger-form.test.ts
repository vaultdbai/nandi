import { describe, expect, it } from 'vitest'
import type { Trigger } from '@/lib/tauri-bindings'
import {
  calendarConfig,
  emptyForm,
  endpointPath,
  formProblems,
  fromTrigger,
  parseList,
  sourceSummary,
  toInput,
  WEEKDAYS,
} from './trigger-form'

const trigger = (overrides: Partial<Trigger>): Trigger => ({
  id: 'contracts',
  name: 'Contracts',
  enabled: true,
  sourceType: 'file',
  sourceConfig: {
    path: '~/Downloads/contracts',
    patterns: ['*.pdf', '*.docx'],
    events: ['create'],
    debounceMs: 500,
    recursive: false,
  },
  targetType: 'agent',
  targetId: 'legal',
  inputTemplate: 'Review {{payload.path}}',
  priority: 70,
  rateLimitSeconds: 60,
  createdAt: 0,
  updatedAt: 0,
  lastFiredAt: null,
  hasSecret: false,
  ...overrides,
})

describe('trigger-form', () => {
  it('parses comma-separated lists', () => {
    expect(parseList(' *.pdf, ,*.docx ,')).toEqual(['*.pdf', '*.docx'])
    expect(parseList('')).toEqual([])
  })

  it('builds the input Rust expects for each source', () => {
    const form = {
      ...emptyForm('file'),
      name: ' Contracts ',
      path: ' ~/in ',
      patterns: '*.pdf, *.docx',
      fileEvents: ['create' as const],
      debounceMs: '500',
      targetId: 'legal',
      rateLimitSeconds: '0',
    }
    expect(toInput(form)).toEqual({
      name: 'Contracts',
      enabled: true,
      sourceType: 'file',
      sourceConfig: {
        path: '~/in',
        patterns: ['*.pdf', '*.docx'],
        events: ['create'],
        debounceMs: 500,
        recursive: false,
      },
      targetType: 'agent',
      targetId: 'legal',
      inputTemplate: null,
      // Empty priority: the source's default
      priority: null,
      rateLimitSeconds: null,
    })

    const device = toInput({
      ...emptyForm('device'),
      deviceId: 'printer-01',
      deviceEvents: 'paper_jam, ink_low',
      priority: '45',
    })
    expect(device.sourceConfig).toEqual({
      deviceId: 'printer-01',
      events: ['paper_jam', 'ink_low'],
    })
    expect(device.priority).toBe(45)

    const event = toInput({
      ...emptyForm('event'),
      filter: [
        { key: 'agent_id', value: 'research' },
        { key: 'status', value: '' },
        { key: '', value: 'x' },
      ],
    })
    expect(event.sourceConfig).toEqual({
      eventName: 'agent.turn_done',
      filter: { agent_id: 'research' },
    })
    expect(toInput(emptyForm('webhook')).sourceConfig).toEqual({})
  })

  it('round-trips an existing trigger through the form', () => {
    const existing = trigger({})
    const form = fromTrigger(existing)
    expect(form.patterns).toBe('*.pdf, *.docx')
    expect(form.fileEvents).toEqual(['create'])
    expect(form.priority).toBe('70')
    expect(toInput(form)).toMatchObject({
      sourceConfig: existing.sourceConfig,
      priority: 70,
      rateLimitSeconds: 60,
      inputTemplate: 'Review {{payload.path}}',
    })
  })

  it('reports what must be filled in', () => {
    expect(formProblems(emptyForm('cron'))).toEqual(['name', 'target'])
    expect(
      formProblems({ ...emptyForm('file'), name: 'x', targetId: 'a' })
    ).toEqual(['path'])
    expect(
      formProblems({ ...emptyForm('device'), name: 'x', targetId: 'a' })
    ).toEqual(['deviceId'])
  })

  it('describes sources and their endpoints', () => {
    expect(sourceSummary(trigger({}))).toBe(
      '~/Downloads/contracts (*.pdf, *.docx)'
    )
    const hook = trigger({ sourceType: 'webhook', sourceConfig: {} })
    expect(endpointPath(hook)).toBe('/api/triggers/webhook/contracts')
    const device = trigger({
      sourceType: 'device',
      sourceConfig: { deviceId: 'printer 01', events: [] },
    })
    expect(endpointPath(device)).toBe('/api/triggers/device/printer%2001')
    expect(endpointPath(trigger({}))).toBeNull()
  })
})

describe('calendar triggers', () => {
  const rota = {
    times: ['09:00'],
    days: ['mon', 'tue', 'wed', 'thu', 'fri'],
    skipDates: ['2026-12-25'],
    people: ['alice', 'bob'],
    rotate: 'day',
    startDate: '2026-10-26',
    byDay: { fri: 'carol' },
    cover: [
      { person: 'dana', from: '2026-11-02', to: '2026-11-06', backup: 'bob' },
    ],
    backup: 'erin',
    backupAfterMinutes: 60,
  }

  it('round-trips a rota with backups through the form', () => {
    const form = fromTrigger(
      trigger({ sourceType: 'calendar', sourceConfig: rota })
    )
    expect(form.people).toBe('alice, bob')
    expect(form.byDay).toEqual({ fri: 'carol' })
    expect(toInput(form).sourceConfig).toEqual(rota)
  })

  it('leaves out what is not set', () => {
    const form = {
      ...emptyForm('calendar'),
      times: '09:00, 17:00',
      days: [...WEEKDAYS],
      byDay: { mon: ' ', tue: 'alice' },
      cover: [{ person: 'dana', from: '', to: '', backup: '' }],
    }
    // Every day is the same as no day filter; a half-filled stand-in is dropped
    expect(calendarConfig(form)).toEqual({
      times: ['09:00', '17:00'],
      byDay: { tue: 'alice' },
    })
  })

  it('needs a time, and a start date when people take turns', () => {
    const form = { ...emptyForm('calendar'), name: 'Rota', targetId: 'a' }
    expect(formProblems(form)).toEqual([])
    expect(formProblems({ ...form, times: ' ' })).toEqual(['times'])
    expect(formProblems({ ...form, people: 'alice' })).toEqual(['startDate'])
  })

  it('summarizes times and people', () => {
    expect(
      sourceSummary(trigger({ sourceType: 'calendar', sourceConfig: rota }))
    ).toBe('09:00 · alice, bob, carol')
  })
})
