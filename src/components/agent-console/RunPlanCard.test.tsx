import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@/test/test-utils'
import type { RunPlan } from '@/lib/bindings'
import { RunPlanCard } from './RunPlanCard'

const mocks = vi.hoisted(() => ({
  getRunPlan: vi.fn(),
  getRunScreenshot: vi.fn(),
  openRunFile: vi.fn(),
}))
vi.mock('@/lib/client', () => ({ client: mocks, isDesktop: () => true }))

const plan: RunPlan = {
  runId: 'r1',
  revision: 2,
  updatedAt: 0,
  steps: [
    {
      title: 'Open acme-energy.com',
      kind: 'navigate',
      evidence: 'screenshot',
      note: null,
      status: 'done',
      result: 'Opened the billing page',
      files: ['browser/r1/1-1.png'],
    },
    {
      title: 'Sign in',
      kind: 'handover',
      evidence: 'none',
      note: 'The user signs in',
      status: 'doing',
      result: null,
      files: [],
    },
    {
      title: 'Read the confirmation',
      kind: 'verify',
      evidence: 'screenshot',
      note: null,
      status: 'pending',
      result: null,
      files: [],
    },
  ],
}

describe('RunPlanCard', () => {
  it('shows each step, its status, result and screenshots', async () => {
    mocks.getRunPlan.mockResolvedValue({ status: 'ok', data: plan })
    mocks.getRunScreenshot.mockResolvedValue({
      status: 'ok',
      data: 'data:image/png;base64,AAAA',
    })
    render(<RunPlanCard runId="r1" />)
    expect(await screen.findByText('Open acme-energy.com')).toBeInTheDocument()
    expect(screen.getByText('1 of 3 done')).toBeInTheDocument()
    expect(screen.getByText('revision 2')).toBeInTheDocument()
    expect(screen.getByText('Opened the billing page')).toBeInTheDocument()
    expect(screen.getByText('You do it')).toBeInTheDocument()
    expect(screen.getByText('The user signs in')).toBeInTheDocument()
    expect(screen.getByText('Doing')).toBeInTheDocument()
    expect(screen.getByText('Screenshot')).toBeInTheDocument()
    expect(
      await screen.findByRole('img', { name: 'Screenshot 1-1.png' })
    ).toBeInTheDocument()
    expect(mocks.getRunScreenshot).toHaveBeenCalledWith(
      'r1',
      'browser/r1/1-1.png'
    )
  })

  it('shows nothing for a run without a plan', async () => {
    mocks.getRunPlan.mockResolvedValue({ status: 'ok', data: null })
    render(<RunPlanCard runId="r2" />)
    await vi.waitFor(() => expect(mocks.getRunPlan).toHaveBeenCalledWith('r2'))
    expect(screen.queryByRole('region', { name: 'Plan' })).toBeNull()
  })
})
