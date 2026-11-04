import { describe, expect, it } from 'vitest'
import source from './bindings.ts?raw'

// bindings.ts is `@ts-nocheck`, so tsc doesn't report two Rust types that
// export under the same name; Vite's transform does, and the app won't load
describe('generated bindings', () => {
  it('export every type name once', () => {
    const names = [...source.matchAll(/^export type (\w+)/gm)].map(m => m[1])
    const duplicates = names.filter((name, i) => names.indexOf(name) !== i)
    expect(duplicates).toEqual([])
  })
})
