import { describe, expect, it } from 'vitest'
import { getField, setField } from './yaml-fields'

const CAP = `---
id: code-review.python
name: Python review   # shown in lists
version: 1.2.3
tags: [python, review]
policy:
  sla_seconds: 300
  approval_required: false
---

Review it.
`

const WORKFLOW = `id: daily
name: Daily
steps:
  - id: one
    agent: writer
`

describe('yaml-fields', () => {
  it('reads scalars, lists, and nested keys', () => {
    expect(getField(CAP, 'name')).toBe('Python review')
    expect(getField(CAP, 'version')).toBe('1.2.3')
    expect(getField(CAP, 'tags')).toEqual(['python', 'review'])
    expect(getField(CAP, 'policy.sla_seconds')).toBe(300)
    expect(getField(CAP, 'policy.approval_required')).toBe(false)
    expect(getField(CAP, 'missing')).toBeUndefined()
    expect(getField(WORKFLOW, 'name')).toBe('Daily')
  })

  it('changes only the edited line', () => {
    const edited = setField(CAP, 'policy.sla_seconds', 60)
    expect(edited).toBe(CAP.replace('sla_seconds: 300', 'sla_seconds: 60'))
    expect(setField(CAP, 'version', '1.3.0')).toBe(
      CAP.replace('version: 1.2.3', 'version: 1.3.0')
    )
    // The body is untouched
    expect(setField(CAP, 'name', 'Py')).toContain('\n\nReview it.\n')
  })

  it('adds missing keys inside the frontmatter and quotes when needed', () => {
    const added = setField(CAP, 'description', 'Reviews: Python')
    expect(added).toContain('description: "Reviews: Python"\n---')
    const nested = setField(CAP, 'routing.strategy', 'fastest')
    expect(nested).toContain('routing:\n  strategy: fastest\n---')
    const child = setField(CAP, 'policy.visibility', 'network')
    expect(getField(child, 'policy.visibility')).toBe('network')
    expect(getField(child, 'policy.sla_seconds')).toBe(300)
    expect(setField(WORKFLOW, 'name', 'true')).toContain('name: "true"')
  })

  it('keeps CRLF files CRLF', () => {
    const crlf = CAP.replace(/\n/g, '\r\n')
    const edited = setField(crlf, 'tags', ['a'])
    expect(edited).toContain('tags: [a]\r\n')
    expect(edited.replace(/\r\n/g, '')).not.toContain('\n')
  })
})
