import { describe, expect, it } from 'vitest'
import en from '../../locales/en.json'
import ar from '../../locales/ar.json'
import fr from '../../locales/fr.json'
import ja from '../../locales/ja.json'
import { helpDestination, helpTopicKeys } from './help'

/** The topic IDs in `src-tauri/src/services/help/guide.json` */
const TOPICS = [
  'install-docker',
  'connect-model',
  'api-key',
  'claude-sign-in',
  'ollama',
  'free-trial',
  'join-relay',
  'agents',
  'groups',
  'inbox',
  'find-your-way',
]

describe('help links', () => {
  it('go to the right screen', () => {
    expect(helpDestination('settings:computer')).toEqual({
      kind: 'view',
      view: { kind: 'settings', section: 'computer' },
    })
    expect(helpDestination('settings:models:anthropic')).toEqual({
      kind: 'view',
      view: { kind: 'settings', section: 'models', focus: 'anthropic' },
    })
    expect(helpDestination('settings')).toEqual({
      kind: 'view',
      view: { kind: 'settings' },
    })
    expect(helpDestination('network:connection')).toEqual({
      kind: 'view',
      view: { kind: 'network', tab: 'connection' },
    })
    expect(helpDestination('inbox')).toEqual({
      kind: 'view',
      view: { kind: 'inbox' },
    })
    expect(helpDestination('url:https://ollama.com/download')).toEqual({
      kind: 'url',
      url: 'https://ollama.com/download',
    })
  })

  it('refuse what this build does not know', () => {
    expect(helpDestination('settings:nowhere')).toBeNull()
    expect(helpDestination('network:elsewhere')).toBeNull()
    expect(helpDestination('url:http://insecure.example')).toBeNull()
    expect(helpDestination('mystery')).toBeNull()
  })
})

describe('help topics', () => {
  it('have a title and answer in every language', () => {
    for (const locale of [en, fr, ar, ja] as Record<string, string>[]) {
      for (const id of TOPICS) {
        const keys = helpTopicKeys(id)
        expect(locale[keys.title], `${id} title`).toBeTruthy()
        expect(locale[keys.answer], `${id} answer`).toBeTruthy()
      }
    }
  })
})
