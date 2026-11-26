import { describe, expect, it } from 'vitest'
import type { AgentInfo, ChatSummary } from '@/lib/bindings'
import {
  chatAgentId,
  chatLabel,
  isLongChat,
  isPhoneChat,
  plainPreview,
  recentChats,
  searchChats,
} from './nandi-home-model'

const chat = (id: string, updatedAt: number, over = {}): ChatSummary => ({
  id,
  agentId: 'nandi',
  title: '',
  createdAt: 0,
  updatedAt,
  lastMessage: null,
  ...over,
})

const nandi = { id: 'nandi', name: 'Nandi' } as AgentInfo

describe('recentChats', () => {
  it('keeps the newest, ten in all, the rest older', () => {
    const chats = Array.from({ length: 12 }, (_, i) => chat(`c${i}`, i))
    const { recent, older } = recentChats(chats, [])
    expect(recent.map(c => c.id)).toEqual([
      'c11',
      'c10',
      'c9',
      'c8',
      'c7',
      'c6',
      'c5',
      'c4',
      'c3',
      'c2',
    ])
    expect(older.map(c => c.id)).toEqual(['c1', 'c0'])
  })

  it('puts pinned chats first and keeps them however old', () => {
    const chats = [chat('old', 1), chat('a', 5), chat('b', 4), chat('c', 3)]
    const { recent, older } = recentChats(chats, ['old'], 3)
    expect(recent.map(c => c.id)).toEqual(['old', 'a', 'b'])
    expect(older.map(c => c.id)).toEqual(['c'])
  })
})

describe('chatLabel', () => {
  it('is the title, or names the long chat, or a new chat', () => {
    expect(chatLabel(chat('chat:1', 0, { title: ' Bills ' }), [nandi])).toEqual(
      { text: 'Bills' }
    )
    expect(chatLabel(chat('agent:nandi', 0), [nandi])).toEqual({
      key: 'nandiHome.longChat',
      values: { name: 'Nandi' },
    })
    expect(chatLabel(chat('chat:2', 0), [nandi])).toEqual({
      key: 'nandiHome.untitled',
    })
  })
})

describe('isLongChat and chatAgentId', () => {
  it('reads the agent of a chat', () => {
    expect(isLongChat(chat('agent:nandi', 0))).toBe(true)
    expect(isLongChat(chat('chat:1', 0))).toBe(false)
    const chats = [chat('chat:1', 0, { agentId: 'writer' })]
    expect(chatAgentId('chat:1', chats)).toBe('writer')
    expect(chatAgentId('agent:reader', chats)).toBe('reader')
    expect(chatAgentId('chat:unknown', chats)).toBeNull()
  })
})

describe('searchChats', () => {
  it('finds chats by title or last message, newest first', () => {
    const chats = [
      chat('a', 1, { title: 'Pay the bills' }),
      chat('b', 3, { lastMessage: 'The BILL is paid' }),
      chat('c', 2, { title: 'Trip' }),
    ]
    expect(searchChats(chats, 'bill').map(c => c.id)).toEqual(['b', 'a'])
    expect(searchChats(chats, '  ').map(c => c.id)).toEqual(['b', 'c', 'a'])
    expect(searchChats(chats, undefined)).toHaveLength(3)
  })
})

describe('plainPreview', () => {
  it('reads Markdown as one line of plain text', () => {
    expect(
      plainPreview(
        '**Morning digest, Thu 2026-11-26**\n1. **Do now:** pay the _gas_ bill\n- see [the bill](https://x.test/b) and `invoice.pdf`'
      )
    ).toBe(
      'Morning digest, Thu 2026-11-26 Do now: pay the gas bill see the bill and invoice.pdf'
    )
    expect(plainPreview('# Title\n> quoted ~~old~~')).toBe('Title quoted old')
    expect(plainPreview('snake_case_name stays')).toBe('snake_case_name stays')
    expect(plainPreview(null)).toBe('')
    expect(plainPreview('x'.repeat(300))).toHaveLength(140)
  })
})

describe('isPhoneChat', () => {
  it('is a chat asked from a paired phone', () => {
    expect(isPhoneChat({ id: 'phone:device.1:abc' })).toBe(true)
    expect(isPhoneChat({ id: 'chat:abc' })).toBe(false)
  })
})
