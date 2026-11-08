import type { AutoApproveRule, IntakeAction, RuleMatch } from '@/lib/bindings'

/** The kinds of rule, in the order the picker lists them */
export const RULE_KINDS: RuleMatch['kind'][] = [
  'emailDomain',
  'emailAddress',
  'teammate',
]

/** The core refuses a longer list */
export const MAX_RULES = 100

const ACTION_ORDER: IntakeAction[] = ['start', 'update', 'run']

const ADDRESS = /^[^\s@<>,;]+@[^\s@<>,;]+\.[^\s@<>,;]+$/
const DOMAIN = /^([a-z0-9]([a-z0-9-]*[a-z0-9])?\.)+[a-z]{2,}$/

/** What a rule of this kind can let through: mail starts or updates work, a teammate's request runs */
export function actionsFor(kind: RuleMatch['kind']): IntakeAction[] {
  return kind === 'teammate' ? ['run'] : ['start', 'update']
}

export function actionKey(action: IntakeAction): string {
  return `autoApprove.action.${action}`
}

/** i18n key and values of a rule's one-line name */
export function ruleLabel(match: RuleMatch): {
  key: string
  values: Record<string, string>
} {
  switch (match.kind) {
    case 'emailDomain':
      return {
        key: 'autoApprove.label.emailDomain',
        values: { domain: match.domain },
      }
    case 'emailAddress':
      return {
        key: 'autoApprove.label.emailAddress',
        values: { address: match.address },
      }
    case 'teammate':
      return {
        key: 'autoApprove.label.teammate',
        values: { name: match.name },
      }
  }
}

export function newRuleId(): string {
  return crypto.randomUUID()
}

/** `Chase.com`, `@chase.com` and ` chase.com ` all mean `chase.com` */
export function normalizeDomain(input: string): string {
  return input.trim().replace(/^@/, '').toLowerCase()
}

export function normalizeAddress(input: string): string {
  return input.trim().toLowerCase()
}

export function isEmailAddress(value: string): boolean {
  return ADDRESS.test(value)
}

export function isDomain(value: string): boolean {
  return DOMAIN.test(value)
}

/** The address in "Name <address>" (or a bare address), lower case */
export function senderAddress(sender: string | null): string | null {
  if (!sender) return null
  const angle = /<([^<>]+)>/.exec(sender)
  const address = normalizeAddress(angle?.[1] ?? sender)
  return isEmailAddress(address) ? address : null
}

/** The domain a sender's mail comes from, when the sender has an address */
export function senderDomain(sender: string | null): string | null {
  const address = senderAddress(sender)
  if (!address) return null
  const domain = address.slice(address.lastIndexOf('@') + 1)
  return isDomain(domain) ? domain : null
}

/** The rule behind "Always allow mail from <domain>" on a queued item */
export function domainShortcut(
  domain: string,
  action: IntakeAction
): AutoApproveRule {
  return {
    id: newRuleId(),
    match: { kind: 'emailDomain', domain },
    actions: [action],
  }
}

/** A teammate to pick: who they are and the ID the rule matches */
export interface TeammateChoice {
  instanceId: string
  name: string
}

/** The "Add rule" form's fields */
export interface RuleDraft {
  kind: RuleMatch['kind']
  /** The address or domain being typed */
  value: string
  teammate: TeammateChoice | null
  actions: IntakeAction[]
}

export function emptyDraft(kind: RuleMatch['kind']): RuleDraft {
  return { kind, value: '', teammate: null, actions: actionsFor(kind) }
}

/** `actions` with one switched on or off, kept in a stable order */
export function toggleAction(
  actions: IntakeAction[],
  action: IntakeAction
): IntakeAction[] {
  const next = actions.includes(action)
    ? actions.filter(a => a !== action)
    : [...actions, action]
  return ACTION_ORDER.filter(a => next.includes(a))
}

/** The draft as a rule, with a fresh ID; `null` while nothing is typed or picked */
export function draftToRule(draft: RuleDraft): AutoApproveRule | null {
  const base = { id: newRuleId(), actions: draft.actions }
  switch (draft.kind) {
    case 'emailDomain': {
      const domain = normalizeDomain(draft.value)
      return domain ? { ...base, match: { kind: 'emailDomain', domain } } : null
    }
    case 'emailAddress': {
      const address = normalizeAddress(draft.value)
      return address
        ? { ...base, match: { kind: 'emailAddress', address } }
        : null
    }
    case 'teammate':
      return draft.teammate
        ? {
            ...base,
            match: {
              kind: 'teammate',
              instanceId: draft.teammate.instanceId,
              name: draft.teammate.name,
            },
          }
        : null
  }
}

/** The i18n key of what is wrong with a rule, as the core would say; `null` when it is fine */
export function checkRule(rule: AutoApproveRule): string | null {
  const allowed = actionsFor(rule.match.kind)
  if (rule.actions.length === 0) return 'autoApprove.error.noActions'
  if (rule.actions.some(action => !allowed.includes(action))) {
    return 'autoApprove.error.wrongAction'
  }
  switch (rule.match.kind) {
    case 'emailAddress':
      return isEmailAddress(rule.match.address)
        ? null
        : 'autoApprove.error.address'
    case 'emailDomain':
      return isDomain(rule.match.domain) ? null : 'autoApprove.error.domain'
    case 'teammate':
      return rule.match.instanceId ? null : 'autoApprove.error.teammate'
  }
}

/** Whether two rules cover the same sender or teammate */
export function sameMatch(a: RuleMatch, b: RuleMatch): boolean {
  switch (a.kind) {
    case 'emailDomain':
      return b.kind === 'emailDomain' && a.domain === b.domain
    case 'emailAddress':
      return b.kind === 'emailAddress' && a.address === b.address
    case 'teammate':
      return b.kind === 'teammate' && a.instanceId === b.instanceId
  }
}

/** Why `rule` can't join `rules` (an i18n key), or `null` when it can */
export function checkNewRule(
  rules: AutoApproveRule[],
  rule: AutoApproveRule
): string | null {
  const own = checkRule(rule)
  if (own) return own
  const merges = rules.some(r => sameMatch(r.match, rule.match))
  return !merges && rules.length >= MAX_RULES ? 'autoApprove.error.full' : null
}

/** `rule` added; a rule for the same sender takes the new actions on top of its own */
export function addRule(
  rules: AutoApproveRule[],
  rule: AutoApproveRule
): AutoApproveRule[] {
  const existing = rules.find(r => sameMatch(r.match, rule.match))
  if (!existing) return [...rules, rule]
  return rules.map(r =>
    r === existing
      ? {
          ...r,
          actions: ACTION_ORDER.filter(
            a => r.actions.includes(a) || rule.actions.includes(a)
          ),
        }
      : r
  )
}

export function removeRule(
  rules: AutoApproveRule[],
  id: string
): AutoApproveRule[] {
  return rules.filter(rule => rule.id !== id)
}

/** The rule's actions with one switched; a rule keeps at least one action */
export function toggleRuleAction(
  rules: AutoApproveRule[],
  id: string,
  action: IntakeAction
): AutoApproveRule[] {
  return rules.map(rule => {
    if (rule.id !== id) return rule
    const actions = toggleAction(rule.actions, action)
    return actions.length === 0 ? rule : { ...rule, actions }
  })
}
