import { describe, it, expect, beforeEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import i18n from '@/i18n/config'
import { commands } from '@/lib/tauri-bindings'
import type { AuditEntry, AuditHealth } from '@/lib/tauri-bindings'
import { useNavigationStore } from '@/store/navigation-store'
import { T0 } from '@/test/fixtures'
import { AuditLogView } from './AuditLogView'

const ok = <T,>(data: T) => ({ status: 'ok' as const, data })
const t = (key: string) => i18n.t(key)

function entry(i: number, overrides: Partial<AuditEntry> = {}): AuditEntry {
  return {
    timestamp: T0 - i * 1_000,
    kind: 'llm',
    agentId: i % 2 === 0 ? 'writer' : 'reader',
    tool: `model-${i}`,
    argsRedacted: { inputTokens: i },
    outcome: i === 1 ? 'error' : 'ok',
    durationMs: 250,
    ...overrides,
  }
}

const healthy: AuditHealth = {
  lostEntries: 0,
  lastError: null,
  chain: { state: 'intact', lines: 3 },
}

const rows = () =>
  within(screen.getByRole('table')).getAllByRole('row').slice(1)

describe('AuditLogView', () => {
  beforeEach(() => {
    useNavigationStore.setState(useNavigationStore.getInitialState())
    commands.getAuditHealth = vi.fn(async () => ok(healthy))
    commands.listAuditLog = vi.fn(async (kind: string) =>
      ok(
        kind === 'llm'
          ? [
              entry(0),
              entry(1, {
                // The backend redacts; the viewer catches what slips by
                argsRedacted: { error: 'Bearer abc123', apiKey: 'sk-live' },
              }),
              entry(2),
            ]
          : []
      )
    )
  })

  it('says nothing about an intact log', async () => {
    render(<AuditLogView tab="llm" />)

    expect(await screen.findByText('model-0')).toBeInTheDocument()
    expect(commands.getAuditHealth).toHaveBeenCalledWith('llm')
    expect(screen.queryByText(t('audit.degraded.title'))).toBeNull()
  })

  it('warns when entries were lost or the chain is broken', async () => {
    commands.getAuditHealth = vi.fn(async () =>
      ok<AuditHealth>({
        lostEntries: 2,
        lastError: 'disk full',
        chain: { state: 'broken', line: 7, reason: 'edited' },
      })
    )
    render(<AuditLogView tab="llm" />)

    expect(
      await screen.findByText(t('audit.degraded.title'))
    ).toBeInTheDocument()
    expect(
      screen.getByText(
        i18n.t('audit.degraded.lost', { count: 2, error: 'disk full' })
      )
    ).toBeInTheDocument()
    expect(
      screen.getByText(i18n.t('audit.degraded.broken', { line: 7 }))
    ).toBeInTheDocument()
  })

  it('reads the tab’s log, newest first, with secrets redacted', async () => {
    render(<AuditLogView tab="llm" />)

    expect(await screen.findByText('model-0')).toBeInTheDocument()
    expect(rows()).toHaveLength(3)
    expect(commands.listAuditLog).toHaveBeenCalledWith('llm', 100, 0)

    const failed = rows()[1] as HTMLElement
    expect(within(failed).getByText('error')).toBeInTheDocument()
    expect(failed).toHaveTextContent('[redacted]')
    expect(failed).not.toHaveTextContent('abc123')
    expect(failed).not.toHaveTextContent('sk-live')
    // Sub-second durations read in ms
    expect(within(failed).getByText('250 ms')).toBeInTheDocument()
  })

  it('searches the page, optionally in one column', async () => {
    const user = userEvent.setup()
    render(<AuditLogView tab="llm" />)
    await screen.findByText('model-0')

    const search = screen.getByRole('searchbox', { name: t('audit.search') })
    await user.type(search, 'reader')
    expect(rows()).toHaveLength(1)
    expect(
      within(rows()[0] as HTMLElement).getByText('model-1')
    ).toBeInTheDocument()

    // "reader" is an agent, not a tool
    await user.selectOptions(
      screen.getByRole('combobox', { name: t('audit.column.label') }),
      'tool'
    )
    expect(screen.queryByRole('table')).not.toBeInTheDocument()
    expect(screen.getByText(t('audit.noMatches'))).toBeInTheDocument()
  })

  it('shows an empty state for a log with no entries', async () => {
    render(<AuditLogView tab="pilots" />)
    expect(await screen.findByText(t('audit.empty.title'))).toBeInTheDocument()
    expect(commands.listAuditLog).toHaveBeenCalledWith('pilots', 100, 0)
  })

  it('shows connector calls in their own tab', async () => {
    commands.listAuditLog = vi.fn(async (kind: string) =>
      ok(
        kind === 'connectors'
          ? [
              entry(0, {
                kind: 'connectors',
                tool: 'filesystem__read_file',
                argsRedacted: { path: '[redacted]' },
              }),
            ]
          : []
      )
    )
    render(<AuditLogView tab="connectors" />)
    expect(await screen.findByText('filesystem__read_file')).toBeInTheDocument()
    expect(commands.listAuditLog).toHaveBeenCalledWith('connectors', 100, 0)
    expect(
      screen.getByRole('tab', { name: t('audit.tab.connectors') })
    ).toHaveAttribute('aria-selected', 'true')
  })

  it('switches logs through navigation', async () => {
    const user = userEvent.setup()
    render(<AuditLogView tab="llm" />)
    await screen.findByText('model-0')

    await user.click(screen.getByRole('tab', { name: t('audit.tab.egress') }))
    // Logs live in Settings now
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'settings',
      section: 'logs',
      logTab: 'egress',
    })
  })

  it('pages on the server', async () => {
    const user = userEvent.setup()
    commands.listAuditLog = vi.fn(
      async (_kind: string, limit: number, offset: number) =>
        ok(
          Array.from({ length: offset === 0 ? limit : 3 }, (_, i) =>
            entry(offset + i)
          )
        )
    )
    render(<AuditLogView tab="llm" />)
    await screen.findByText('model-0')
    expect(rows()).toHaveLength(100)

    await user.click(screen.getByRole('button', { name: t('audit.next') }))
    await waitFor(() => expect(rows()).toHaveLength(3))
    expect(commands.listAuditLog).toHaveBeenLastCalledWith('llm', 100, 100)
    expect(screen.getByRole('button', { name: t('audit.next') })).toBeDisabled()
  })
})
