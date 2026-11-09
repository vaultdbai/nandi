import { describe, expect, it } from 'vitest'
import { NARROW_WINDOW_PX, paneLayout } from './layout-model'

describe('paneLayout', () => {
  it('shows both panes on a wide window when nothing is saved', () => {
    expect(paneLayout(undefined, 1400)).toEqual({ sidebar: true, tray: true })
  })

  it('starts the tray hidden on a narrow window until the user picks', () => {
    expect(paneLayout({}, NARROW_WINDOW_PX - 1).tray).toBe(false)
    expect(paneLayout({ workTrayVisible: true }, 600).tray).toBe(true)
  })

  it('keeps what the user picked, whatever the width', () => {
    expect(
      paneLayout({ sidebarVisible: false, workTrayVisible: false }, 1400)
    ).toEqual({ sidebar: false, tray: false })
  })
})
