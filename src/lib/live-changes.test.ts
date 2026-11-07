import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { LiveChange } from '@/lib/bindings'
import { commands } from '@/lib/tauri-bindings'
import { handleNandiEvent } from './agent-sync'
import {
  applyLiveChange,
  FILE_BACKED_KEYS,
  liveQueryKeys,
  queryKeysFor,
  refreshFromDisk,
} from './live-changes'
import { queryClient } from './query-client'

const change = (over: Partial<LiveChange> = {}): LiveChange => ({
  id: 'c1',
  area: 'notes',
  targetId: 'deadlines.md',
  label: 'deadlines.md',
  paths: ['deadlines.md'],
  pathCount: 1,
  action: 'updated',
  author: { kind: 'outside' },
  at: 1_000,
  ...over,
})

/** Query keys invalidated so far */
const invalidated = (spy: ReturnType<typeof vi.spyOn>): unknown[] =>
  (spy.mock.calls as unknown[][]).map(
    ([filters]) => (filters as { queryKey: unknown }).queryKey
  )

describe('live changes', () => {
  let invalidate: ReturnType<typeof vi.spyOn>

  beforeEach(() => {
    invalidate = vi.spyOn(queryClient, 'invalidateQueries')
  })

  afterEach(() => {
    invalidate.mockRestore()
    queryClient.clear()
  })

  it.each([
    [
      change(),
      [
        ['notes', 'list'],
        ['notes', 'note', 'deadlines.md'],
      ],
    ],
    [change({ targetId: null }), [['notes']]],
    [
      change({ area: 'agent', targetId: 'writer' }),
      [
        ['agent-definition', 'writer'],
        ['agent-policies', 'writer'],
      ],
    ],
    [
      change({ area: 'workflow', targetId: 'weekly' }),
      [['workflow', 'weekly']],
    ],
    [change({ area: 'trigger', targetId: 'cron' }), [['trigger', 'cron']]],
    [change({ area: 'policy' }), [['agent-policies'], ['policies']]],
    [change({ area: 'settings', targetId: null }), [['preferences']]],
    [change({ area: 'connectors', targetId: null }), [['connectors']]],
    [change({ area: 'packs', targetId: null }), [['marketplace']]],
    [
      change({ area: 'profile', targetId: null }),
      [
        ['gateway', 'manifest'],
        ['gateway', 'config'],
        ['gateway', 'status'],
      ],
    ],
    [change({ area: 'work', targetId: 'writer' }), []],
  ])('%o invalidates exactly its queries', (live, keys) => {
    expect(queryKeysFor(live)).toEqual(keys)
  })

  it('live://changed refetches what changed and tops the recent list', () => {
    queryClient.setQueryData(liveQueryKeys.recent, [change({ id: 'old' })])
    handleNandiEvent({ event: 'live://changed', payload: change() })
    expect(invalidated(invalidate)).toEqual([
      ['notes', 'list'],
      ['notes', 'note', 'deadlines.md'],
    ])
    const recent = queryClient.getQueryData<LiveChange[]>(liveQueryKeys.recent)
    expect(recent?.map(c => c.id)).toEqual(['c1', 'old'])
  })

  it('applying the same change twice keeps one entry', () => {
    queryClient.setQueryData(liveQueryKeys.recent, [])
    applyLiveChange(change())
    applyLiveChange(change())
    expect(queryClient.getQueryData(liveQueryKeys.recent)).toHaveLength(1)
  })

  it('leaves an unloaded recent list for its first fetch', () => {
    applyLiveChange(change())
    expect(queryClient.getQueryData(liveQueryKeys.recent)).toBeUndefined()
  })

  it('Refresh re-reads the disk, then every file-backed screen', async () => {
    const refresh = vi.fn().mockResolvedValue({
      status: 'ok',
      data: { changed: true },
    })
    Object.assign(commands, { refreshFromDisk: refresh })
    await expect(refreshFromDisk()).resolves.toEqual({ changed: true })
    expect(refresh).toHaveBeenCalledOnce()
    const keys = invalidated(invalidate)
    for (const key of FILE_BACKED_KEYS) expect(keys).toContainEqual(key)
    expect(keys).toContainEqual(['notes'])
    expect(keys).toContainEqual(['agents'])
    expect(keys).toContainEqual(['preferences'])
  })

  it('a failed refresh says why and refetches nothing', async () => {
    Object.assign(commands, {
      refreshFromDisk: vi
        .fn()
        .mockResolvedValue({ status: 'error', error: 'disk gone' }),
    })
    await expect(refreshFromDisk()).rejects.toThrow('disk gone')
    expect(invalidate).not.toHaveBeenCalled()
  })
})
