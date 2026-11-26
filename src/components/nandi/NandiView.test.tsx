import { describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { DEFAULT_AGENT } from '@/lib/bindings'
import { NandiView } from './NandiView'

describe('NandiView', () => {
  it('starts a chat from an example ask, with the box at the bottom', async () => {
    const user = userEvent.setup()
    render(<NandiView />)
    const example = await screen.findByRole('button', {
      name: "What's waiting for me?",
    })
    const box = screen.getByPlaceholderText('What should I do?')
    // The examples sit right above the box, which comes last
    expect(
      example.compareDocumentPosition(box) & Node.DOCUMENT_POSITION_FOLLOWING
    ).toBeTruthy()
    await user.click(example)
    await vi.waitFor(() =>
      expect(
        vi.mocked(commands.sendChatMessage).mock.calls.at(-1)?.slice(0, 2)
      ).toEqual(['chat:new', "What's waiting for me?"])
    )
    expect(commands.createChat).toHaveBeenCalledWith(DEFAULT_AGENT)
  })

  it('shows Nandi with its bison and a plain preview in Recent', async () => {
    vi.mocked(commands.listChats).mockResolvedValue({
      status: 'ok',
      data: [
        {
          id: 'chat:c1',
          agentId: DEFAULT_AGENT,
          title: 'Morning digest',
          createdAt: 0,
          updatedAt: 1,
          lastMessage: '**Do now:** pay the _gas_ bill',
        },
      ],
    })
    render(<NandiView />)
    expect(
      await screen.findByText('Do now: pay the gas bill')
    ).toBeInTheDocument()
    expect(screen.queryByText('NA')).toBeNull()
  })
})
