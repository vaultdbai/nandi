import { describe, expect, it, vi } from 'vitest'
import { useState } from 'react'
import userEvent from '@testing-library/user-event'
import { fireEvent, render, screen } from '@/test/test-utils'
import { ClarificationPolicyEditor } from '@/components/agent-console/ClarificationPolicyEditor'
import { DEFAULT_CLARIFICATION_POLICY } from '@/components/agents/clarification-policy'
import type { ClarificationPolicy } from '@/lib/bindings'

function Harness({ onChange }: { onChange: (p: ClarificationPolicy) => void }) {
  const [value, setValue] = useState(DEFAULT_CLARIFICATION_POLICY)
  return (
    <ClarificationPolicyEditor
      value={value}
      onChange={next => {
        setValue(next)
        onChange(next)
      }}
    />
  )
}

describe('ClarificationPolicyEditor', () => {
  it('round-trips every field through onChange', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<Harness onChange={onChange} />)

    fireEvent.change(screen.getByLabelText(/Rounds of questions per task/), {
      target: { value: '4' },
    })
    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ maxRounds: 4 })
    )

    const [auto, escalate] = screen.getAllByPlaceholderText('Add a topic…')
    await user.type(auto as HTMLElement, 'formatting{Enter}')
    await user.type(escalate as HTMLElement, 'budget{Enter}')

    const timeout = screen.getByLabelText('Answer within (seconds)')
    fireEvent.change(timeout, { target: { value: '600' } })

    expect(onChange).toHaveBeenLastCalledWith({
      allow: true,
      maxRounds: 4,
      autoTopics: ['formatting'],
      escalateTopics: ['budget'],
      timeoutSeconds: 600,
    })

    await user.click(screen.getByRole('switch'))
    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ allow: false })
    )
    expect(timeout).toBeDisabled()
  })

  it('renders the value it is given', () => {
    render(
      <ClarificationPolicyEditor
        value={{
          allow: true,
          maxRounds: 1,
          autoTopics: ['dates'],
          escalateTopics: [],
          timeoutSeconds: 120,
        }}
        onChange={vi.fn()}
      />
    )
    expect(screen.getByText('dates')).toBeInTheDocument()
    expect(screen.getByLabelText('Answer within (seconds)')).toHaveValue(120)
    expect(
      screen.getByText('Rounds of questions per task: 1')
    ).toBeInTheDocument()
  })
})
