import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import '@/i18n'
import { CompanionCollapsed } from './CompanionCollapsed'

function bison(onDragStart?: () => void) {
  const onExpand = vi.fn()
  render(
    <CompanionCollapsed
      orb={{ state: 'needsYou', badge: 2 }}
      pose="wait"
      tooltip="2 need you"
      onExpand={onExpand}
      onDragStart={onDragStart}
    />
  )
  return { onExpand, button: screen.getByRole('button') }
}

describe('CompanionCollapsed', () => {
  it('shows the pose and the count, and a click opens the card', () => {
    const { onExpand, button } = bison(vi.fn())
    expect(button).toHaveAttribute('data-pose', 'wait')
    expect(screen.getByTestId('orb-badge')).toHaveTextContent('2')
    fireEvent.pointerDown(button, { button: 0, clientX: 10, clientY: 10 })
    fireEvent.pointerMove(button, { clientX: 12, clientY: 11 })
    fireEvent.pointerUp(button)
    fireEvent.click(button)
    expect(onExpand).toHaveBeenCalledOnce()
  })

  it('a press that moves drags the window instead of opening the card', () => {
    const onDragStart = vi.fn()
    const { onExpand, button } = bison(onDragStart)
    fireEvent.pointerDown(button, { button: 0, clientX: 10, clientY: 10 })
    fireEvent.pointerMove(button, { clientX: 30, clientY: 10 })
    fireEvent.pointerMove(button, { clientX: 60, clientY: 10 })
    fireEvent.click(button)
    expect(onDragStart).toHaveBeenCalledOnce()
    expect(onExpand).not.toHaveBeenCalled()
    // The next plain click opens it
    fireEvent.pointerDown(button, { button: 0, clientX: 60, clientY: 10 })
    fireEvent.click(button)
    expect(onExpand).toHaveBeenCalledOnce()
  })

  it('without a window to drag, a moving press is still a click', () => {
    const { onExpand, button } = bison()
    fireEvent.pointerDown(button, { button: 0, clientX: 10, clientY: 10 })
    fireEvent.pointerMove(button, { clientX: 60, clientY: 10 })
    fireEvent.click(button)
    expect(onExpand).toHaveBeenCalledOnce()
  })
})
