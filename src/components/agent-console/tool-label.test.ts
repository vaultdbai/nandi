import { describe, expect, it } from 'vitest'
import i18n from '@/i18n/config'
import { toolLabel } from './tool-label'

const t = i18n.t.bind(i18n)

describe('toolLabel', () => {
  it('says what a known tool does, with its detail', () => {
    expect(
      toolLabel(
        'filesystem__read_file',
        { path: 'my-notes/responsibilities.md' },
        t
      )
    ).toBe('Read my-notes/responsibilities.md')
    expect(toolLabel('find_people', { topic: 'SOC 1' }, t)).toBe(
      'Find who is responsible: SOC 1'
    )
    expect(
      toolLabel('github__get_pull_request', { url: 'acme/web#12' }, t)
    ).toBe('Read pull request acme/web#12')
  })

  it('reads plainly when the detail is missing', () => {
    expect(toolLabel('list_dir', {}, t)).toBe('Look through the shared folders')
    expect(toolLabel('my_requests', {}, t)).toBe('Check your open requests')
    expect(toolLabel('delegate_to_agent', { target: 'x' }, t)).toBe(
      'Send a request to another agent'
    )
  })

  it('names AWS actions, and words any other tool', () => {
    expect(toolLabel('aws__batch_describe', { job_ids: ['j-1'] }, t)).toBe(
      'AWS: Batch describe'
    )
    expect(toolLabel('shell', { command: 'ls' }, t)).toBe('Shell')
    expect(toolLabel('acme__open_ticket', null, t)).toBe('Open ticket')
  })

  it('says what web search does, not just "Search"', () => {
    expect(toolLabel('web_search__search', { query: 'tokio select' }, t)).toBe(
      'Search the web: tokio select'
    )
    expect(
      toolLabel('web_search__read_page', { url: 'https://docs.rs/tokio' }, t)
    ).toBe('Read https://docs.rs/tokio')
    expect(toolLabel('web_search__read_page', {}, t)).toBe('Read a web page')
    // Another connector's `search` is still just its name
    expect(toolLabel('notes__search', { query: 'x' }, t)).toBe('Search')
  })

  it('says what Studio looks at, never its tool IDs', () => {
    expect(toolLabel('nandi_get_agent', { id: 'writer' }, t)).toBe(
      'Agent details: writer'
    )
    expect(toolLabel('nandi_get_agent', {}, t)).toBe('Look at an agent')
    expect(toolLabel('nandi_list_runs', { window: '7d' }, t)).toBe(
      'Look at recent runs'
    )
    expect(toolLabel('nandi_save_artifact', { kind: 'agent' }, t)).toBe(
      'Save the draft'
    )
    // Not a labelled tool, though every object has it
    expect(toolLabel('toString', null, t)).toBe('ToString')
  })
})
