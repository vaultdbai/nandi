import { describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import type { HostRule } from '@/lib/tauri-bindings'
import { NetworkAccess } from './NetworkAccess'

const show = (
  sites: string[],
  rules: HostRule[] = sites.map(host => ({ host, methods: [] }))
) => {
  const onChange = vi.fn()
  render(<NetworkAccess sites={sites} rules={rules} onChange={onChange} />)
  return onChange
}

describe('NetworkAccess', () => {
  it('starts on what the agent has', () => {
    show([])
    expect(screen.getByLabelText('No internet')).toBeChecked()
  })

  it('allows any site with *', async () => {
    const onChange = show(['api.example.com'])
    await userEvent.click(screen.getByLabelText('Any site'))
    expect(onChange).toHaveBeenLastCalledWith(['*'])
    expect(screen.getByText(/can reach any website/)).toBeInTheDocument()

    // Back to the list it had
    await userEvent.click(screen.getByLabelText('Only these sites'))
    expect(onChange).toHaveBeenLastCalledWith(['api.example.com'])
  })

  it('adds, changes and removes sites', async () => {
    const onChange = show(['a.example.com'])
    expect(screen.getByLabelText('Only these sites')).toBeChecked()

    await userEvent.type(
      screen.getByLabelText('Add a site'),
      'API.GitHub.com{Enter}'
    )
    expect(onChange).toHaveBeenLastCalledWith([
      'a.example.com',
      'api.github.com',
    ])

    const first = screen.getByLabelText('Site 1')
    await userEvent.clear(first)
    await userEvent.type(first, '*.example.com')
    expect(onChange).toHaveBeenLastCalledWith([
      '*.example.com',
      'api.github.com',
    ])

    await userEvent.click(screen.getByLabelText('Remove api.github.com'))
    expect(onChange).toHaveBeenLastCalledWith(['*.example.com'])
  })

  it('refuses bad and duplicate sites', async () => {
    const onChange = show(['a.example.com'])
    const add = screen.getByLabelText('Add a site')

    await userEvent.type(add, 'a.*.com{Enter}')
    expect(screen.getByRole('alert')).toHaveTextContent(/Use a site name/)

    await userEvent.clear(add)
    await userEvent.type(add, 'A.example.com{Enter}')
    expect(screen.getByRole('alert')).toHaveTextContent(/already in the list/)
    expect(onChange).not.toHaveBeenCalled()
  })

  it('shows the methods a site is limited to', () => {
    show(['api.github.com'], [{ host: 'api.github.com', methods: ['GET'] }])
    expect(screen.getByText('GET only')).toBeInTheDocument()
  })
})
