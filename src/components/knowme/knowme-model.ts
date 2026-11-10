/**
 * A know-me proposal (`knowme_propose`, `services::knowme::proposal`): what
 * its card shows and edits before the person confirms it. Pure. Rust checks
 * the confirmed proposal again (personal items off the card, a group only
 * one I'm in), so this only shapes what the card shows.
 */
import type {
  GroupRef,
  KnowMeProposal,
  ProposedResponsibility,
} from '@/lib/bindings'

/** The tool whose approval is a know-me proposal */
export const KNOW_ME_PROPOSE_TOOL = 'knowme_propose'

type Record_ = Record<string, unknown>

const isRecord = (value: unknown): value is Record_ =>
  typeof value === 'object' && value !== null && !Array.isArray(value)

const text = (value: unknown): string =>
  typeof value === 'string' ? value.trim() : ''

const optional = (value: unknown): string | null => text(value) || null

const records = (value: unknown): Record_[] =>
  Array.isArray(value) ? value.filter(isRecord) : []

/** The proposal in a `knowme_propose` approval, or `null` for another tool */
export function parseKnowMeProposal(
  tool: string,
  args: unknown
): KnowMeProposal | null {
  if (tool !== KNOW_ME_PROPOSE_TOOL || !isRecord(args)) return null
  return {
    summary: text(args.summary),
    responsibilities: records(args.responsibilities)
      .map(r => {
        const personal = r.kind === 'personal'
        return {
          text: text(r.text),
          kind: personal ? 'personal' : 'work',
          // Work is on the card unless switched off; personal never is
          onCard: personal ? false : r.onCard !== false,
          shareWithGroup: personal ? optional(r.shareWithGroup) : null,
        } satisfies ProposedResponsibility
      })
      .filter(r => r.text !== ''),
    description: optional(args.description),
    canHelpWith: optional(args.canHelpWith),
    cantHelpWith: optional(args.cantHelpWith),
    notes: records(args.notes)
      .map(n => ({ text: text(n.text), private: n.private === true }))
      .filter(n => n.text !== ''),
    offer: Array.isArray(args.offer)
      ? args.offer.map(text).filter(name => name !== '')
      : [],
    suggest: records(args.suggest)
      .map(s => ({ name: text(s.name), why: text(s.why) }))
      .filter(s => s.name !== ''),
  }
}

/** A personal item's group, only if it's one I'm in */
export function groupName(
  groups: GroupRef[],
  id: string | null
): string | null {
  if (!id) return null
  return groups.find(g => g.id === id)?.name ?? null
}

/** The proposal with one responsibility changed */
export function withResponsibility(
  proposal: KnowMeProposal,
  index: number,
  change: Partial<ProposedResponsibility>
): KnowMeProposal {
  return {
    ...proposal,
    responsibilities: proposal.responsibilities.map((item, i) => {
      if (i !== index) return item
      const next = { ...item, ...change }
      // The rules the card can't break: personal never on the card, a group
      // only on a personal item
      return next.kind === 'personal'
        ? { ...next, onCard: false }
        : { ...next, shareWithGroup: null }
    }),
  }
}

/** The proposal without one responsibility, note or suggestion */
export function without(
  proposal: KnowMeProposal,
  part: 'responsibilities' | 'notes' | 'offer' | 'suggest',
  index: number
): KnowMeProposal {
  return {
    ...proposal,
    [part]: (proposal[part] as unknown[]).filter((_, i) => i !== index),
  }
}

/** Whether confirming would change anything */
export function changesSomething(proposal: KnowMeProposal): boolean {
  return (
    proposal.responsibilities.some(r => r.text.trim() !== '') ||
    proposal.description !== null ||
    proposal.canHelpWith !== null ||
    proposal.cantHelpWith !== null ||
    proposal.notes.length > 0 ||
    proposal.offer.length > 0 ||
    proposal.suggest.length > 0
  )
}
