import { describe, expect, it } from 'vitest'
import { normalizeSecretName, secretNameProblem } from './site-secrets-model'

describe('site secrets model', () => {
  it('types names in capitals with underscores', () => {
    expect(normalizeSecretName('acme password')).toBe('ACME_PASSWORD')
    expect(normalizeSecretName('  acme-energy.com')).toBe('ACME_ENERGY_COM')
    expect(normalizeSecretName('SITE_')).toBe('SITE_')
  })

  it('says why a name cannot be added', () => {
    expect(secretNameProblem('', [])).toBe('empty')
    expect(secretNameProblem('1PASSWORD', [])).toBe('format')
    expect(secretNameProblem('_X', [])).toBe('format')
    expect(secretNameProblem('SITE_PASSWORD', ['SITE_PASSWORD'])).toBe('taken')
    expect(secretNameProblem('ACME_PASSWORD', ['SITE_PASSWORD'])).toBeNull()
  })
})
