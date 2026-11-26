import { describe, expect, it } from 'vitest'
import { saveDraft } from './save-preview-model'

describe('saveDraft', () => {
  it('is the file to open in the editor', () => {
    expect(
      saveDraft({ kind: 'agent', id: 'researcher', content: '---\n---\n' })
    ).toEqual({
      artifact: { kind: 'agent', id: 'researcher' },
      before: '',
      after: '---\n---\n',
      diff: '',
      impact: [],
      versionBump: null,
    })
  })

  it('takes the ID the preview read, for a workflow named by its file', () => {
    expect(
      saveDraft({ kind: 'workflow', content: 'id: bills' }, 'bills')?.artifact
    ).toEqual({ kind: 'workflow', id: 'bills' })
  })

  it('is nothing for what the editor does not open', () => {
    expect(saveDraft({ kind: 'trigger', trigger: {} })).toBeNull()
    expect(saveDraft({ kind: 'agent', content: 'x' })).toBeNull()
    expect(saveDraft(null)).toBeNull()
    expect(saveDraft([])).toBeNull()
  })
})
