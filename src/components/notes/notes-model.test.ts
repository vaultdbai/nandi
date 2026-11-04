import { describe, expect, it } from 'vitest'
import type { NoteFile } from '@/lib/bindings'
import {
  groupNotes,
  newNoteContent,
  newNoteName,
  noteLabel,
} from './notes-model'

const note = (name: string, title: string | null = null): NoteFile => ({
  name,
  title,
  updatedAt: 1,
  size: 10,
})

describe('groupNotes', () => {
  it('puts top-level notes first, then each folder, sorted', () => {
    const groups = groupNotes([
      note('runbooks/etl.md'),
      note('owners.md'),
      note('archive/2025.md'),
      note('deadlines.md'),
    ])
    expect(groups.map(g => g.folder)).toEqual(['', 'archive', 'runbooks'])
    expect(groups[0]?.notes.map(n => n.name)).toEqual([
      'deadlines.md',
      'owners.md',
    ])
  })
})

describe('noteLabel', () => {
  it('shows the heading, else the file name', () => {
    expect(noteLabel(note('owners.md', 'Who owns what'))).toBe('Who owns what')
    expect(noteLabel(note('runbooks/etl-nightly.md'))).toBe('etl-nightly')
  })
})

describe('newNoteName', () => {
  it('adds .md and keeps folders', () => {
    expect(newNoteName(' deadlines ')).toBe('deadlines.md')
    expect(newNoteName('runbooks/etl nightly')).toBe('runbooks/etl nightly.md')
    expect(newNoteName('Q4 plan.md')).toBe('Q4 plan.md')
    expect(newNoteName('runbooks\\etl')).toBe('runbooks/etl.md')
  })

  it('refuses names that could leave the folder or hide', () => {
    for (const bad of [
      '',
      '   ',
      '../secrets',
      'a/../b',
      '.hidden',
      'a//b',
      'a/b/c/d',
      'what?',
    ]) {
      expect(newNoteName(bad), bad).toBeNull()
    }
  })
})

describe('newNoteContent', () => {
  it('starts with the name as a heading', () => {
    expect(newNoteContent('runbooks/etl-nightly.md')).toBe('# Etl nightly\n\n')
  })
})
