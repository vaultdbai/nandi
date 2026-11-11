import { describe, expect, it } from 'vitest'
import { render, screen } from '@/test/test-utils'
import { OfferAbout } from './OfferView'

describe('About this Nandi', () => {
  it('renders NANDI.md as markdown, never raw headings', () => {
    render(
      <OfferAbout about={'## What I can help with\n\n- Research papers'} />
    )
    expect(
      screen.getByRole('heading', { name: 'What I can help with' })
    ).toBeInTheDocument()
    expect(screen.getByRole('listitem')).toHaveTextContent('Research papers')
    expect(screen.getByTestId('offer-about')).not.toHaveTextContent('##')
  })

  it('says so when there is nothing to show', () => {
    render(<OfferAbout about="" />)
    expect(screen.queryByTestId('offer-about')).toBeNull()
  })
})
