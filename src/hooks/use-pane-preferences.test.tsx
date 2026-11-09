import { beforeEach, describe, expect, it, vi } from 'vitest'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook, waitFor } from '@testing-library/react'
import { commands } from '@/lib/tauri-bindings'
import { useUIStore } from '@/store/ui-store'
import { usePanePreferences } from './use-pane-preferences'

function renderPanes() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return renderHook(() => usePanePreferences(), {
    wrapper: ({ children }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    ),
  })
}

const width = (px: number) =>
  Object.defineProperty(window, 'innerWidth', {
    value: px,
    configurable: true,
  })

describe('usePanePreferences', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    useUIStore.setState({ leftSidebarVisible: true, rightSidebarVisible: true })
    width(1400)
  })

  it('applies the saved layout', async () => {
    vi.mocked(commands.loadPreferences).mockResolvedValueOnce({
      status: 'ok',
      data: { theme: 'system', language: null, sidebarVisible: false },
    })
    renderPanes()
    await waitFor(() =>
      expect(useUIStore.getState().leftSidebarVisible).toBe(false)
    )
    expect(useUIStore.getState().rightSidebarVisible).toBe(true)
    expect(commands.savePreferences).not.toHaveBeenCalled()
  })

  it('starts the tray hidden on a narrow window and follows it until picked', async () => {
    width(800)
    renderPanes()
    await waitFor(() =>
      expect(useUIStore.getState().rightSidebarVisible).toBe(false)
    )
    act(() => {
      width(1400)
      window.dispatchEvent(new Event('resize'))
    })
    expect(useUIStore.getState().rightSidebarVisible).toBe(true)
    expect(commands.savePreferences).not.toHaveBeenCalled()
  })

  it('saves a show or hide, and then stops following the width', async () => {
    useUIStore.setState({ rightSidebarVisible: false })
    renderPanes()
    // Applied once loaded: a wide window shows the tray
    await waitFor(() =>
      expect(useUIStore.getState().rightSidebarVisible).toBe(true)
    )
    act(() => useUIStore.getState().toggleRightSidebar())
    await waitFor(() =>
      expect(commands.savePreferences).toHaveBeenCalledWith(
        expect.objectContaining({
          sidebarVisible: true,
          workTrayVisible: false,
        })
      )
    )
    act(() => {
      width(1600)
      window.dispatchEvent(new Event('resize'))
    })
    expect(useUIStore.getState().rightSidebarVisible).toBe(false)
  })
})
