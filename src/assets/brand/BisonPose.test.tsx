import { describe, expect, it } from 'vitest'
import { render } from '@testing-library/react'
import { BisonPose } from './BisonPose'

const parts = (container: HTMLElement) =>
  Array.from(container.querySelectorAll('img')).map(img =>
    Array.from(img.classList)
      .find(c => c.startsWith('bp-') && c !== 'bp-part')
      ?.slice(3)
  )

describe('BisonPose', () => {
  it.each([
    ['rest', ['base', 'head', 'stand']],
    ['wait', ['base', 'head', 'tail', 'badge']],
    ['carry', ['base', 'head', 'tail', 'crates']],
    ['done', ['base', 'head', 'tail', 'badge']],
  ] as const)('%s stacks the parts that move', (pose, expected) => {
    const { container } = render(<BisonPose pose={pose} />)
    expect(container.firstElementChild).toHaveAttribute('data-pose', pose)
    expect(parts(container)).toEqual(expected)
  })

  it('is decorative: the caller labels it', () => {
    const { container } = render(<BisonPose pose="rest" />)
    expect(container.firstElementChild).toHaveAttribute('aria-hidden', 'true')
    for (const img of container.querySelectorAll('img')) {
      expect(img).toHaveAttribute('alt', '')
    }
  })
})
