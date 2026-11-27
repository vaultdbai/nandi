/**
 * Examples that teach: a pool of about 30 asks Nandi can really do, each
 * tagged with what it teaches. A few show at a time, a new set each time the
 * Nandi home or a new chat opens; what the person hasn't tried comes first,
 * and an ask that needs something not connected yet becomes the step that
 * connects it. Pure; the words are in the locale files (`examples.<id>`).
 */

/** What an example teaches */
export type ExampleKind =
  | 'mail'
  | 'calendar'
  | 'browser'
  | 'automations'
  | 'agents'
  | 'lockBox'
  | 'phone'
  | 'people'
  | 'general'

/** What an example needs connected first */
export type ExampleNeed = 'mail' | 'relay' | 'phone'

export interface Example {
  id: string
  kind: ExampleKind
  needs?: ExampleNeed
}

/** The pool, in no order (each `examples.<id>` in every locale) */
export const EXAMPLES: readonly Example[] = [
  { id: 'mail1', kind: 'mail', needs: 'mail' },
  { id: 'mail2', kind: 'mail', needs: 'mail' },
  { id: 'mail3', kind: 'mail', needs: 'mail' },
  { id: 'mail4', kind: 'mail', needs: 'mail' },
  { id: 'cal1', kind: 'calendar', needs: 'mail' },
  { id: 'cal2', kind: 'calendar', needs: 'mail' },
  { id: 'cal3', kind: 'calendar', needs: 'mail' },
  { id: 'cal4', kind: 'calendar', needs: 'mail' },
  { id: 'web1', kind: 'browser' },
  { id: 'web2', kind: 'browser' },
  { id: 'web3', kind: 'browser' },
  { id: 'web4', kind: 'browser' },
  { id: 'auto1', kind: 'automations' },
  { id: 'auto2', kind: 'automations' },
  { id: 'auto3', kind: 'automations' },
  { id: 'auto4', kind: 'automations' },
  { id: 'agent1', kind: 'agents' },
  { id: 'agent2', kind: 'agents' },
  { id: 'agent3', kind: 'agents' },
  { id: 'key1', kind: 'lockBox' },
  { id: 'key2', kind: 'lockBox' },
  { id: 'phone1', kind: 'phone', needs: 'phone' },
  { id: 'phone2', kind: 'phone', needs: 'phone' },
  { id: 'people1', kind: 'people', needs: 'relay' },
  { id: 'people2', kind: 'people', needs: 'relay' },
  { id: 'people3', kind: 'people', needs: 'relay' },
  { id: 'gen1', kind: 'general' },
  { id: 'gen2', kind: 'general' },
  { id: 'gen3', kind: 'general' },
  { id: 'gen4', kind: 'general' },
]

/** How many show at a time */
export const SHOWN = 4

/** One chip: an ask to send, or the step that connects what it needs */
export type ExampleChip =
  { kind: 'ask'; example: Example } | { kind: 'connect'; need: ExampleNeed }

export interface ExampleContext {
  /** What's connected now */
  connected: ReadonlySet<ExampleNeed>
  /** Kinds the person has used: they step back */
  used: ReadonlySet<ExampleKind>
  /** The ids the last set showed: not shown again back to back */
  previous: readonly string[]
}

/** A small seeded generator (mulberry32), so a seed gives the same set */
function random(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

function shuffled<T>(items: readonly T[], seed: number): T[] {
  const next = random(seed)
  const out = [...items]
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.floor(next() * (i + 1))
    const swap = out[i] as T
    out[i] = out[j] as T
    out[j] = swap
  }
  return out
}

/**
 * The set to show for `seed`: untried kinds first, one per kind, none of
 * the last set's when there are enough others; an ask whose need isn't
 * connected becomes that need's connect step (once)
 */
export function pickExamples(
  seed: number,
  context: ExampleContext,
  count = SHOWN
): ExampleChip[] {
  const fresh = EXAMPLES.filter(e => !context.previous.includes(e.id))
  const pool = fresh.length >= count ? fresh : [...EXAMPLES]
  const order = shuffled(pool, seed).sort(
    (a, b) =>
      Number(context.used.has(a.kind)) - Number(context.used.has(b.kind))
  )
  const kinds = new Set<ExampleKind>()
  const picked: Example[] = []
  for (const example of order) {
    if (picked.length === count) break
    if (kinds.has(example.kind)) continue
    kinds.add(example.kind)
    picked.push(example)
  }
  const needs = new Set<ExampleNeed>()
  const chips: ExampleChip[] = []
  for (const example of picked) {
    const need = example.needs
    if (need && !context.connected.has(need)) {
      if (needs.has(need)) continue
      needs.add(need)
      chips.push({ kind: 'connect', need })
    } else {
      chips.push({ kind: 'ask', example })
    }
  }
  return chips
}

/** The ids a set showed, for the next set to leave out */
export function shownIds(chips: readonly ExampleChip[]): string[] {
  return chips.flatMap(chip => (chip.kind === 'ask' ? [chip.example.id] : []))
}
