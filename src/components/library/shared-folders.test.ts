import { describe, expect, it } from 'vitest'
import {
  chooseFolder,
  effectiveAccess,
  folderConfig,
  folderIdFrom,
  folderProblems,
  foldersOf,
  type SharedFolder,
} from './shared-folders'

const docs: SharedFolder = { id: 'docs', path: '/d', access: 'read' }
const reports: SharedFolder = { id: 'reports', path: '/r', access: 'write' }

describe('foldersOf', () => {
  it('reads the list', () => {
    expect(
      foldersOf({
        folders: [
          { id: 'docs', path: '/d', access: 'read' },
          { id: 'reports', path: '/r', access: 'write' },
          { id: 'odd' },
        ],
      })
    ).toEqual([docs, reports])
  })

  it('reads no folders without a list', () => {
    expect(foldersOf({ folders: 'docs' })).toEqual([])
    expect(foldersOf({})).toEqual([])
    expect(foldersOf(undefined)).toEqual([])
  })

  it('round-trips through the config', () => {
    const config = folderConfig([{ ...docs, id: ' docs ' }, reports])
    expect(config).toEqual({ folders: [docs, reports] })
    expect(foldersOf(config)).toEqual([docs, reports])
  })
})

describe('folderIdFrom', () => {
  it('makes a short unique ID from the folder name', () => {
    expect(folderIdFrom('C:\\Users\\a\\Work Notes', [])).toBe('work-notes')
    expect(folderIdFrom('/home/a/docs/', ['docs'])).toBe('docs-2')
    expect(folderIdFrom('/', [])).toBe('folder')
  })
})

describe('folderProblems', () => {
  it('names the first problem per row', () => {
    expect(
      folderProblems([
        docs,
        { ...reports, path: ' ' },
        { ...reports, id: 'Bad ID' },
        { ...reports, id: 'docs' },
      ])
    ).toEqual([null, 'noPath', 'badId', 'duplicateId'])
  })
})

describe('an agent’s folders', () => {
  it('never gets more access than the folder has', () => {
    expect(effectiveAccess(docs, { id: 'docs', access: 'write' })).toBe('read')
    expect(effectiveAccess(reports, { id: 'reports' })).toBe('write')
    expect(effectiveAccess(reports, { id: 'reports', access: 'read' })).toBe(
      'read'
    )
  })

  it('turns folders on, narrows them, and turns them off', () => {
    let choices = chooseFolder([], reports, 'write')
    expect(choices).toEqual([{ id: 'reports', access: null }])
    choices = chooseFolder(choices, docs, 'read')
    expect(choices).toEqual([
      { id: 'reports', access: null },
      { id: 'docs', access: null },
    ])
    choices = chooseFolder(choices, reports, 'read')
    expect(choices[0]).toEqual({ id: 'reports', access: 'read' })
    expect(chooseFolder(choices, docs, 'off')).toEqual([
      { id: 'reports', access: 'read' },
    ])
  })
})
