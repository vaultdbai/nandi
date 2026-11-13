/**
 * The mascot's state from what the app is doing, and the hidden phrase.
 * The phrase is checked by hash, so its words aren't in the bundle; the
 * core writes the reply (`mascotPhrase`), which is only ever a line of
 * text: it can't approve, run or route work.
 */
import type { NandiState } from '@/assets/brand/NandiBull'

/** Lowercase words, punctuation dropped, single spaces */
export function normalizePhrase(input: string): string {
  return input
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, ' ')
    .trim()
}

/** 32-bit FNV-1a over UTF-8 */
export function fnv1a(text: string): number {
  let hash = 0x811c9dc5
  for (const byte of new TextEncoder().encode(text)) {
    hash = Math.imul(hash ^ byte, 0x01000193) >>> 0
  }
  return hash
}

/** The phrase's two spellings, hashed */
const PHRASES = new Set([0x07fd06eb, 0x3a9b45e3])

/** Whether `input` is the hidden phrase (any case, punctuation ignored) */
export function isMascotPhrase(input: string): boolean {
  return PHRASES.has(fnv1a(normalizePhrase(input)))
}

/** The bull's pose: waiting on you first, then working, else resting */
export function nandiState(input: {
  pendingApprovals: number
  runningAgents: number
  doneUntil: number
  now: number
}): NandiState {
  if (input.now < input.doneUntil) return 'done'
  if (input.pendingApprovals > 0) return 'wait'
  if (input.runningAgents > 0) return 'carry'
  return 'rest'
}
