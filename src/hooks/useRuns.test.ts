import { describe, it, expect, vi } from 'vitest'
import { commands } from '@/lib/tauri-bindings'
import type { RunPage } from '@/lib/tauri-bindings'
import { run, T0 } from '@/test/fixtures'
import {
  fetchAllRuns,
  hasRunFilters,
  normalizeRunFilters,
  OPEN_END,
  runsToCsv,
  statsRange,
} from './useRuns'

const ok = <T>(data: T) => ({ status: 'ok' as const, data })

describe('run filter parsing', () => {
  it('drops blank values and trims strings', () => {
    expect(
      normalizeRunFilters({
        status: '',
        sourceType: '  trigger ',
        agentId: null,
        search: '   ',
        dateFrom: T0,
        dateTo: null,
      })
    ).toEqual({ sourceType: 'trigger', dateFrom: T0 })
  })

  it('ignores dates that are not finite numbers', () => {
    expect(normalizeRunFilters({ dateFrom: Number.NaN, dateTo: 5 })).toEqual({
      dateTo: 5,
    })
  })

  it('gives equal filters an equal shape (one query key)', () => {
    expect(normalizeRunFilters({ agentId: 'writer', search: '' })).toEqual(
      normalizeRunFilters({ search: undefined, agentId: ' writer' })
    )
  })

  it('knows whether anything narrows the result', () => {
    expect(hasRunFilters({})).toBe(false)
    expect(hasRunFilters({ status: '', search: ' ' })).toBe(false)
    expect(hasRunFilters({ status: 'failed' })).toBe(true)
  })

  it('takes the stats range from the date filters, open-ended otherwise', () => {
    expect(statsRange({})).toEqual({ from: 0, to: OPEN_END })
    expect(statsRange({ dateFrom: 10, dateTo: 20 })).toEqual({
      from: 10,
      to: 20,
    })
  })
})

describe('runsToCsv', () => {
  it('writes a header and one quoted row per run', () => {
    const csv = runsToCsv(
      [
        run('r1', 'writer', {
          input: 'Say "hi", then\nleave',
          status: 'completed',
          endedAt: T0 + 2_500,
          inputTokens: 12,
          outputTokens: 3,
          costUsd: 0.01,
          model: 'claude:claude-sonnet-5',
        }),
      ],
      { writer: 'Writer' }
    )
    const [header, row] = csv.split('\r\n')
    expect(header).toBe(
      'id,startedAt,endedAt,durationMs,agentId,agentName,status,sourceType,sourceLabel,input,summary,model,inputTokens,outputTokens,costUsd'
    )
    // The embedded newline stays inside the quoted cell
    expect(row).toBe(
      `r1,${new Date(T0).toISOString()},${new Date(T0 + 2_500).toISOString()},2500,writer,Writer,completed,user,,"Say ""hi"", then\nleave",,claude:claude-sonnet-5,12,3,0.01`
    )
  })

  it('defuses spreadsheet formulas', () => {
    const csv = runsToCsv([run('r1', 'writer', { input: '=HYPERLINK("x")' })])
    expect(csv).toContain(`"'=HYPERLINK(""x"")"`)
  })
})

describe('fetchAllRuns', () => {
  it('pages until it has every matching run', async () => {
    const all = Array.from({ length: 450 }, (_, i) => run(`r${i}`, 'writer'))
    commands.listAllRuns = vi.fn(
      async (_filters, limit: number, offset: number) =>
        ok<RunPage>({ rows: all.slice(offset, offset + limit), total: 450 })
    )

    const rows = await fetchAllRuns({ status: 'failed', search: '' })

    expect(rows).toHaveLength(450)
    expect(vi.mocked(commands.listAllRuns).mock.calls).toEqual([
      [{ status: 'failed' }, 200, 0],
      [{ status: 'failed' }, 200, 200],
      [{ status: 'failed' }, 200, 400],
    ])
  })

  it('stops at the cap', async () => {
    const all = Array.from({ length: 50 }, (_, i) => run(`r${i}`, 'writer'))
    commands.listAllRuns = vi.fn(
      async (_filters, limit: number, offset: number) =>
        ok<RunPage>({ rows: all.slice(offset, offset + limit), total: 50 })
    )
    expect(await fetchAllRuns({}, 20)).toHaveLength(20)
  })
})
