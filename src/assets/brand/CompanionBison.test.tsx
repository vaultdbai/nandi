import { describe, expect, it } from 'vitest'
import { render } from '@testing-library/react'
import { CompanionBison } from './CompanionBison'

describe('CompanionBison', () => {
  it('has the moving image and the still frame, chosen by the motion setting', () => {
    const { container } = render(<CompanionBison />)
    expect(container.firstElementChild).toHaveAttribute('aria-hidden')
    expect(container.querySelector('.companion-bison-moving')).toHaveAttribute(
      'src',
      expect.stringContaining('companion-bison.webp')
    )
    expect(container.querySelector('.companion-bison-still')).toHaveAttribute(
      'src',
      expect.stringContaining('companion-bison-still')
    )
  })
})
