import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

// Mock the Tauri menu API. Hoisted so the module can be imported statically
// below: a dynamic import inside the first test could exceed the 5s timeout
// under load.
const { mockPopup, mockMenuNew, mockMenuItemNew, mockPredefinedMenuItemNew } =
  vi.hoisted(() => {
    const mockPopup = vi.fn().mockResolvedValue(undefined)
    return {
      mockPopup,
      mockMenuNew: vi.fn().mockResolvedValue({ popup: mockPopup }),
      mockMenuItemNew: vi.fn().mockResolvedValue({}),
      mockPredefinedMenuItemNew: vi.fn().mockResolvedValue({}),
    }
  })

vi.mock('@tauri-apps/api/menu', () => ({
  Menu: {
    new: mockMenuNew,
  },
  MenuItem: {
    new: mockMenuItemNew,
  },
  PredefinedMenuItem: {
    new: mockPredefinedMenuItemNew,
  },
}))

import {
  installContextMenuGuard,
  showContextMenu,
  showEditContextMenu,
  showTextInputContextMenu,
} from './context-menu'

describe('context-menu utilities', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('should export all utility functions', () => {
    expect(showContextMenu).toBeDefined()
    expect(showEditContextMenu).toBeDefined()
    expect(showTextInputContextMenu).toBeDefined()
  })

  it('showContextMenu should create menu items and show popup', async () => {
    const action = vi.fn()
    await showContextMenu([
      { id: 'test', label: 'Test Item', action },
      { type: 'separator' },
      { id: 'disabled', label: 'Disabled', disabled: true },
      { id: 'shortcut', label: 'With Shortcut', accelerator: 'CmdOrCtrl+K' },
    ])

    // Should create MenuItem for regular items
    expect(mockMenuItemNew).toHaveBeenCalledWith({
      id: 'test',
      text: 'Test Item',
      accelerator: undefined,
      enabled: true,
      action,
    })

    // Should create PredefinedMenuItem for separator
    expect(mockPredefinedMenuItemNew).toHaveBeenCalledWith({
      item: 'Separator',
    })

    // Should create disabled MenuItem
    expect(mockMenuItemNew).toHaveBeenCalledWith({
      id: 'disabled',
      text: 'Disabled',
      accelerator: undefined,
      enabled: false,
      action: undefined,
    })

    // Should pass accelerator through
    expect(mockMenuItemNew).toHaveBeenCalledWith({
      id: 'shortcut',
      text: 'With Shortcut',
      accelerator: 'CmdOrCtrl+K',
      enabled: true,
      action: undefined,
    })

    // Should create menu and show popup
    expect(mockMenuNew).toHaveBeenCalled()
    expect(mockPopup).toHaveBeenCalled()
  })

  it('showEditContextMenu should create standard edit menu', async () => {
    await showEditContextMenu()

    // Should create Cut, Copy, Paste, Separator, SelectAll
    expect(mockPredefinedMenuItemNew).toHaveBeenCalledWith({ item: 'Cut' })
    expect(mockPredefinedMenuItemNew).toHaveBeenCalledWith({ item: 'Copy' })
    expect(mockPredefinedMenuItemNew).toHaveBeenCalledWith({ item: 'Paste' })
    expect(mockPredefinedMenuItemNew).toHaveBeenCalledWith({
      item: 'Separator',
    })
    expect(mockPredefinedMenuItemNew).toHaveBeenCalledWith({
      item: 'SelectAll',
    })
    expect(mockPopup).toHaveBeenCalled()
  })

  it('showTextInputContextMenu should include Undo/Redo', async () => {
    await showTextInputContextMenu()

    // Should include Undo and Redo
    expect(mockPredefinedMenuItemNew).toHaveBeenCalledWith({ item: 'Undo' })
    expect(mockPredefinedMenuItemNew).toHaveBeenCalledWith({ item: 'Redo' })
    expect(mockPopup).toHaveBeenCalled()
  })
})

describe('installContextMenuGuard', () => {
  let remove: () => void

  const rightClick = (target: EventTarget) => {
    const event = new MouseEvent('contextmenu', {
      bubbles: true,
      cancelable: true,
    })
    target.dispatchEvent(event)
    return event
  }

  beforeEach(() => {
    vi.clearAllMocks()
    remove = installContextMenuGuard()
  })

  afterEach(() => {
    remove()
    document.body.innerHTML = ''
  })

  it('blocks the browser menu (and its OS Share) on plain content', () => {
    const div = document.body.appendChild(document.createElement('div'))
    expect(rightClick(div).defaultPrevented).toBe(true)
    expect(mockPopup).not.toHaveBeenCalled()
  })

  it('shows the text menu in an editable field', async () => {
    const input = document.body.appendChild(document.createElement('input'))
    expect(rightClick(input).defaultPrevented).toBe(true)
    await vi.waitFor(() => expect(mockPopup).toHaveBeenCalledOnce())
  })

  it('offers Copy for selected text that cannot be edited', async () => {
    const p = document.body.appendChild(document.createElement('p'))
    p.textContent = 'An agent reply'
    window.getSelection()?.selectAllChildren(p)
    expect(rightClick(p).defaultPrevented).toBe(true)
    await vi.waitFor(() => expect(mockPopup).toHaveBeenCalledOnce())
    expect(mockPredefinedMenuItemNew).toHaveBeenCalledWith({ item: 'Copy' })
    expect(mockPredefinedMenuItemNew).not.toHaveBeenCalledWith({ item: 'Cut' })
    window.getSelection()?.removeAllRanges()
  })

  it('leaves elements with their own menu alone', () => {
    const div = document.body.appendChild(document.createElement('div'))
    div.addEventListener('contextmenu', event => event.preventDefault())
    rightClick(div)
    expect(mockPopup).not.toHaveBeenCalled()
  })

  it('stops guarding once removed', () => {
    remove()
    remove = () => undefined
    const div = document.body.appendChild(document.createElement('div'))
    expect(rightClick(div).defaultPrevented).toBe(false)
  })
})
