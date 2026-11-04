import { describe, expect, it, vi } from 'vitest'
import { act, renderHook, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { useArtifact, usePushTargets, useSaveArtifact } from './useStudio'

const { mockClient } = vi.hoisted(() => ({
  mockClient: {
    loadArtifact: vi.fn(),
    saveArtifact: vi.fn(),
    listPushTargets: vi.fn(),
    subscribe: vi.fn(() => ({
      ready: Promise.resolve(),
      unsubscribe: vi.fn(),
    })),
  },
}))

vi.mock('@/lib/client', () => ({ client: mockClient }))

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  )
}

const workflow = { kind: 'workflow', id: 'daily' } as const

describe('useStudio', () => {
  it('loads an artifact and reloads it after a save', async () => {
    mockClient.loadArtifact.mockResolvedValue({
      status: 'ok',
      data: {
        artifact: workflow,
        content: 'id: daily\n',
        parsed: { frontmatter: { id: 'daily' }, body: '' },
        path: '/repo/workflows/daily.yaml',
      },
    })
    mockClient.saveArtifact.mockResolvedValue({
      status: 'ok',
      data: {
        artifact: workflow,
        commit: 'abc',
        pushedBranch: null,
        warnings: [],
      },
    })
    const { result } = renderHook(
      () => ({ artifact: useArtifact(workflow), save: useSaveArtifact() }),
      { wrapper }
    )
    await waitFor(() =>
      expect(result.current.artifact.data?.content).toBe('id: daily\n')
    )
    expect(mockClient.loadArtifact).toHaveBeenCalledWith(workflow)

    await act(() =>
      result.current.save.mutateAsync({
        artifact: workflow,
        content: 'id: daily\nname: Daily\n',
        commitMessage: 'Name it',
        pushTarget: 'localOnly',
      })
    )
    await waitFor(() =>
      expect(mockClient.loadArtifact).toHaveBeenCalledTimes(2)
    )
  })

  it('stays idle without an artifact and lists push targets', async () => {
    mockClient.loadArtifact.mockClear()
    mockClient.listPushTargets.mockResolvedValue({
      status: 'ok',
      data: [
        { target: 'localOnly', available: true, branch: null, reason: null },
      ],
    })
    const { result } = renderHook(
      () => ({ artifact: useArtifact(null), targets: usePushTargets() }),
      { wrapper }
    )
    await waitFor(() => expect(result.current.targets.data).toHaveLength(1))
    expect(result.current.artifact.fetchStatus).toBe('idle')
    expect(mockClient.loadArtifact).not.toHaveBeenCalled()
  })
})
