import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@/test/test-utils'
import { getField } from '@/lib/yaml-fields'
import { PolicyForm } from './PolicyForm'

const TEXT = "---\nname: finance\napplies_to: ['*']\n---\n\nWhy.\n"

describe('PolicyForm', () => {
  it('writes rule lists into the frontmatter', () => {
    const onChange = vi.fn()
    render(<PolicyForm text={TEXT} onChange={onChange} />)
    fireEvent.change(screen.getByLabelText('Never these tools'), {
      target: { value: 'shell, write_file' },
    })
    const written = onChange.mock.calls.at(-1)?.[0] as string
    expect(getField(written, 'tools.deny')).toEqual(['shell', 'write_file'])
    expect(getField(written, 'applies_to')).toEqual(['*'])
  })

  it('shows the rules a file already has', () => {
    render(
      <PolicyForm
        text={"---\napplies_to: ['*']\nmodels:\n  allow: ['claude:*']\n---\n"}
        onChange={vi.fn()}
      />
    )
    expect(
      screen.getByLabelText('Only these models (e.g. claude:*)')
    ).toHaveValue('claude:*')
  })

  it('says which targets work, and that workflow targets are refused', () => {
    render(<PolicyForm text={TEXT} onChange={vi.fn()} />)
    expect(screen.getByLabelText('Applies to')).toHaveAccessibleDescription(
      /Workflow targets aren't supported yet/
    )
  })
})
