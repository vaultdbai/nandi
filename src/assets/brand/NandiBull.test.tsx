import { describe, expect, it } from 'vitest'
import { render, screen } from '@/test/test-utils'
import { ThemeProviderContext } from '@/lib/theme-context'
import { MASCOT_POSES, PALETTE_ICONS } from '@/themes/palette-icons'
import { PALETTES } from '@/themes/palettes'
import { NandiBull } from './NandiBull'

describe('NandiBull', () => {
  it('shows the current palette icon and its state', () => {
    render(
      <ThemeProviderContext.Provider
        value={{
          theme: 'dark',
          setTheme: () => null,
          palette: 'ocean',
          setPalette: () => null,
        }}
      >
        <NandiBull state="wait" name="Nandu" size={26} />
      </ThemeProviderContext.Provider>
    )
    const bull = screen.getByRole('img')
    expect(bull).toHaveAttribute('data-state', 'wait')
    expect(bull.querySelector('img')).toHaveAttribute(
      'src',
      PALETTE_ICONS.ocean
    )
  })

  it('shows the whole bison in its pose from 48 px up', () => {
    render(<NandiBull state="carry" size={72} />)
    const bull = screen.getByRole('img')
    expect(bull.querySelector('img')).toHaveAttribute('src', MASCOT_POSES.carry)
    expect(bull.querySelector('.nb-alert')).toBeNull()
  })

  it('has an icon for every palette', () => {
    for (const { id } of PALETTES) expect(PALETTE_ICONS[id]).toBeTruthy()
  })
})
