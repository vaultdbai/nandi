import { describe, expect, it, beforeEach } from 'vitest'
import { waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { commands } from '@/lib/tauri-bindings'
import { render, screen } from '@/test/test-utils'
import { useAgentStore } from '@/store/agent-store'
import { useNavigationStore } from '@/store/navigation-store'
import { StudioView } from './StudioView'

const conversation = (id: string, title: string, updatedAt: number) => ({
  id,
  title,
  createdAt: 0,
  updatedAt,
  lastMessage: `${title} last message`,
})

describe('StudioView', () => {
  beforeEach(() => {
    useAgentStore.setState(useAgentStore.getInitialState())
    useNavigationStore.setState(useNavigationStore.getInitialState())
  })

  it('offers New and example asks when there are no conversations', () => {
    useAgentStore.getState().setStudio([])
    render(<StudioView conversationId={null} />)
    expect(
      screen.getByRole('button', { name: /^(New|studio\.newMenu)$/ })
    ).toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: "What's waiting for me?" })
    ).toBeInTheDocument()
  })

  it('opens a new chat from the New menu', async () => {
    useAgentStore.getState().setStudio([])
    const user = userEvent.setup()
    render(<StudioView conversationId={null} />)

    // From the keyboard: in jsdom every rect is empty, so the resize
    // handle claims a pointer press anywhere in the panel group
    screen.getByRole('button', { name: 'New' }).focus()
    await user.keyboard('{Enter}')
    await user.click(await screen.findByRole('menuitem', { name: 'Chat' }))
    // An empty chat: nothing is created until its first message
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'studio',
      conversationId: null,
      draft: true,
    })
    expect(commands.createStudioConversation).not.toHaveBeenCalled()
  })

  it('a new chat invites the first message instead of listing chats', () => {
    useAgentStore.getState().setStudio([conversation('old', 'Old chat', 1)])
    render(<StudioView conversationId={null} draft />)
    expect(screen.queryByRole('button', { name: /Old chat/ })).toBeNull()
    expect(
      screen.getByRole('button', { name: "What's waiting for me?" })
    ).toBeInTheDocument()
    expect(commands.createStudioConversation).not.toHaveBeenCalled()
  })

  it('lists conversations, newest first, when none is selected', async () => {
    useAgentStore
      .getState()
      .setStudio([
        conversation('old', 'Old chat', 1),
        conversation('new', 'Research', 2),
      ])
    const user = userEvent.setup()
    render(<StudioView conversationId={null} />)

    const list = screen.getByRole('list')
    const items = list.querySelectorAll('li')
    expect(items[0]).toHaveTextContent('Research')
    expect(items[1]).toHaveTextContent('Old chat')
    expect(
      screen.getByRole('button', { name: /^(New|studio\.newMenu)$/ })
    ).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: /Research/ }))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'studio',
      conversationId: 'new',
    })
  })

  it('approves a Studio approval card right in the chat', async () => {
    useAgentStore.getState().setStudio([conversation('c1', 'Save it', 1)])
    useAgentStore.getState().setConversation({
      conversationId: 'c1',
      messages: [
        {
          kind: 'approval',
          id: 'ap1',
          runId: 'r1',
          createdAt: Date.now(),
          tool: 'nandi_save_artifact',
          args: {},
          status: 'pending',
          expiresAt: null,
        },
      ],
      currentRun: null,
    })
    const user = userEvent.setup()
    render(<StudioView conversationId="c1" />)

    const approve = screen.getByRole('button', { name: 'Approve' })
    // From the keyboard: the resize handle claims pointer presses in jsdom
    approve.focus()
    await user.keyboard('{Enter}')
    await waitFor(() =>
      expect(commands.approveAction).toHaveBeenCalledWith(
        '@studio',
        'ap1',
        'approved'
      )
    )
    // Waiting for Rust's approval_resolved: no double submit
    expect(approve).toBeDisabled()
  })
})
