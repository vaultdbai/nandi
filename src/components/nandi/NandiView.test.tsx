import { describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { DEFAULT_AGENT } from '@/lib/bindings'
import en from '../../../locales/en.json'
import { pickExamples } from './examples-model'
import { NandiView } from './NandiView'

describe('NandiView', () => {
  it('starts a chat from an example ask, with the box at the bottom', async () => {
    // A seed whose set has an ask with nothing connected (as in the tests)
    const nothing = {
      connected: new Set<never>(),
      used: new Set<never>(),
      previous: [],
    }
    const value = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9].find(v =>
      pickExamples(Math.floor(v * 2 ** 31), nothing).some(c => c.kind === 'ask')
    ) as number
    vi.spyOn(Math, 'random').mockReturnValue(value)
    const chips = pickExamples(Math.floor(value * 2 ** 31), nothing)
    const first = chips.find(c => c.kind === 'ask')
    if (first?.kind !== 'ask') throw new Error('no ask')
    const text =
      (en as Record<string, string>)[`examples.${first.example.id}`] ?? ''

    const user = userEvent.setup()
    render(<NandiView />)
    const example = await screen.findByRole('button', { name: text })
    const box = screen.getByPlaceholderText('What should I do?')
    // The examples sit right above the box, which comes last
    expect(
      example.compareDocumentPosition(box) & Node.DOCUMENT_POSITION_FOLLOWING
    ).toBeTruthy()
    await user.click(example)
    await vi.waitFor(() =>
      expect(
        vi.mocked(commands.sendChatMessage).mock.calls.at(-1)?.slice(0, 2)
      ).toEqual(['chat:new', text])
    )
    expect(commands.createChat).toHaveBeenCalledWith(DEFAULT_AGENT)
    vi.mocked(Math.random).mockRestore()
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
