import { describe, expect, it } from 'vitest'
import en from '../../locales/en.json'
import ar from '../../locales/ar.json'
import fr from '../../locales/fr.json'
import ja from '../../locales/ja.json'
import { availableLanguages } from './config'
import { languageForLocale } from './language-init'

const locales: Record<string, Record<string, string>> = { ar, fr, ja }

const placeholders = (value: string) =>
  (value.match(/\{\{\s*[\w.]+\s*\}\}/g) ?? [])
    .map(p => p.replace(/\s/g, ''))
    .sort()

describe('locales', () => {
  it('registers every locale file', () => {
    expect([...availableLanguages].sort()).toEqual(['ar', 'en', 'fr', 'ja'])
  })

  it.each(Object.keys(locales))('%s has exactly the English keys', lang => {
    expect(Object.keys(locales[lang] ?? {}).sort()).toEqual(
      Object.keys(en).sort()
    )
  })

  it.each(Object.keys(locales))('%s keeps every placeholder', lang => {
    const locale = locales[lang] ?? {}
    const mismatched = Object.entries(en).filter(
      ([key, value]) =>
        placeholders(value).join() !== placeholders(locale[key] ?? '').join()
    )
    expect(mismatched.map(([key]) => key)).toEqual([])
  })
})

describe('languageForLocale', () => {
  it.each([
    ['ja-JP', 'ja'],
    ['ja_JP', 'ja'],
    ['JA', 'ja'],
    ['fr-CA', 'fr'],
    ['ar-EG', 'ar'],
    ['en-GB', 'en'],
    ['de-DE', 'en'],
    ['', 'en'],
    [null, 'en'],
  ])('%s → %s', (locale, expected) => {
    expect(languageForLocale(locale)).toBe(expected)
  })
})
