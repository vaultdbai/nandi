/**
 * What an approval asks, in plain words, and how long it has left. Pure:
 * the card, the Inbox and Run Detail all render from these.
 */

import type { ActionPlan } from '@/lib/bindings'
import {
  ACCESS_TOOL,
  agentLabel,
  parseAccessRequest,
} from '@/components/connectors/access-model'
import {
  parsePlaybookProposal,
  PROPOSE_PLAYBOOK_TOOL,
} from '@/components/agent-console/playbook-proposal-model'

type Args = unknown

/** An i18n key under `approval.summary.` and its values */
export interface ApprovalSummary {
  key:
    | 'shell'
    | 'writeFile'
    | 'readFile'
    | 'listDir'
    | 'delegate'
    | 'delegateTo'
    | 'startRequest'
    | 'saveArtifact'
    | 'pause'
    | 'postReview'
    | 'batchRestart'
    | 'ecsRedeploy'
    | 'connector'
    | 'connectorAccess'
    | 'siteAccess'
    | 'handOver'
    | 'pilotSetup'
    | 'proposePlaybook'
    | 'updatePlaybook'
    | 'other'
  values: Record<string, string>
}

/** Studio starting a Request (`studio::start::START_REQUEST`) */
const START_REQUEST_TOOL = 'nandi_start_request'

/** The plan a `nandi_start_request` approval would start, checked by Rust */
export function approvalPlan(tool: string, args: Args): ActionPlan | null {
  if (tool !== START_REQUEST_TOOL) return null
  if (typeof args !== 'object' || args === null) return null
  const plan = (args as Record<string, unknown>).plan
  if (typeof plan !== 'object' || plan === null) return null
  return Array.isArray((plan as ActionPlan).steps) ? (plan as ActionPlan) : null
}

/** The browser hand-over (`pilots::handover`): a person finishes the page */
const HAND_OVER_TOOL = 'pilot_browser__browser_hand_over'

export function isHandOver(tool: string): boolean {
  return tool === HAND_OVER_TOOL
}

/** What a hand-over shows: where, what's ready, and the text as typed */
export function handOverDraft(args: Args): {
  site: string
  summary: string
  draft: string
} {
  const draft =
    typeof args === 'object' && args !== null
      ? (args as Record<string, unknown>).draft
      : null
  return {
    site: field(args, 'site') ?? '',
    summary: field(args, 'summary') ?? '',
    draft: typeof draft === 'string' ? draft : '',
  }
}

/**
 * A declared browser that isn't installed offers `pilot_<id>__setup`
 * (`pilots::tools::SETUP_TOOL`): approving it installs the browser
 */
const PILOT_SETUP_TOOL = /^pilot_(browser|browser-headless)__setup$/

/** The pilot's catalog name (product names, not translated) */
const PILOT_NAMES: Record<string, string> = {
  browser: 'Browser',
  'browser-headless': 'Browser (headless)',
}

/** Studio saving a definition (`studio::save::SAVE_ARTIFACT`) */
const SAVE_ARTIFACT_TOOL = 'nandi_save_artifact'

/** The pause a middleware asks for (`orchestrator::PAUSE_TOOL`) */
const PAUSE_TOOL = 'continue_run'
/** Longest argument shown in the sentence; the JSON has the rest */
const MAX_VALUE_CHARS = 120

function field(args: Args, ...names: string[]): string | null {
  if (typeof args !== 'object' || args === null) return null
  for (const name of names) {
    const value = (args as Record<string, unknown>)[name]
    if (typeof value === 'string' && value.trim()) return value.trim()
  }
  return null
}

function short(text: string): string {
  const oneLine = text.replace(/\s+/g, ' ')
  return oneLine.length > MAX_VALUE_CHARS
    ? `${oneLine.slice(0, MAX_VALUE_CHARS)}…`
    : oneLine
}

/** `shell {cmd: "git push"}` → "Run a command: `git push`" */
export function approvalSummary(tool: string, args: Args): ApprovalSummary {
  const value = (...names: string[]) => short(field(args, ...names) ?? '')
  switch (tool) {
    case 'shell':
      return { key: 'shell', values: { cmd: value('cmd', 'command') } }
    case 'write_file':
      return { key: 'writeFile', values: { path: value('path') } }
    case 'read_file':
      return { key: 'readFile', values: { path: value('path') } }
    case 'list_dir':
      return { key: 'listDir', values: { path: value('path') || '.' } }
    case 'delegate_to_agent':
      return delegateSummary(args, value('capability', 'task'))
    case START_REQUEST_TOOL:
      return { key: 'startRequest', values: { text: value('text') } }
    case SAVE_ARTIFACT_TOOL: {
      const trigger =
        typeof args === 'object' && args !== null
          ? (args as Record<string, unknown>).trigger
          : null
      return {
        key: 'saveArtifact',
        values: {
          kind: value('kind'),
          id: value('id') || short(field(trigger, 'id', 'name') ?? ''),
        },
      }
    }
    case PAUSE_TOOL:
      return { key: 'pause', values: { reason: value('reason') } }
    case ACCESS_TOOL: {
      const request = parseAccessRequest(args)
      if (!request) break
      if (request.site) {
        return {
          key: 'siteAccess',
          values: {
            agent: agentLabel(request.agent),
            site: request.site.display,
          },
        }
      }
      return {
        key: 'connectorAccess',
        values: {
          agent: agentLabel(request.agent),
          names: request.connectors.map(c => c.name).join(', '),
        },
      }
    }
    case HAND_OVER_TOOL:
      return {
        key: 'handOver',
        values: { site: value('site'), summary: value('summary') },
      }
  }
  const setup = PILOT_SETUP_TOOL.exec(tool)?.[1]
  if (setup) {
    return {
      key: 'pilotSetup',
      values: { pilot: PILOT_NAMES[setup] ?? setup, reason: value('reason') },
    }
  }
  // The connector actions the demo agents ask for, in their own words
  switch (tool) {
    case PROPOSE_PLAYBOOK_TOOL:
      return {
        key: parsePlaybookProposal(tool, args)?.current
          ? 'updatePlaybook'
          : 'proposePlaybook',
        values: { name: value('name') },
      }
    case 'github__post_review':
      return { key: 'postReview', values: { url: value('url') } }
    case 'aws__batch_restart':
      return { key: 'batchRestart', values: { job: value('job_id') } }
    case 'aws__ecs_redeploy':
      return {
        key: 'ecsRedeploy',
        values: { service: value('service'), cluster: value('cluster') },
      }
  }
  const connector = /^(.+?)__(.+)$/.exec(tool)
  if (connector?.[1] && connector[2]) {
    return {
      key: 'connector',
      values: { connector: connector[1], tool: connector[2] },
    }
  }
  return { key: 'other', values: { tool } }
}

/**
 * `delegate_to_agent {capability, target: "<instance>/<agent>", input}` →
 * who it goes to and what it asks. `who` is the target until the card
 * names the person (`delegateWho`).
 */
function delegateSummary(args: Args, capability: string): ApprovalSummary {
  const target = field(args, 'target')
  const input =
    typeof args === 'object' && args !== null
      ? (args as Record<string, unknown>).input
      : null
  const text = short(
    field(input, 'text') ??
      (input === null || input === undefined ? '' : JSON.stringify(input))
  )
  if (!target) {
    return { key: 'delegate', values: { capability } }
  }
  const [instance = target, agent = ''] = target.split('/')
  return {
    key: 'delegateTo',
    values: {
      capability,
      text: text || capability,
      instance,
      agent,
      who: target,
    },
  }
}

/**
 * The person a delegation goes to, by name when they're a known network
 * member: their gateway is them, any other agent is "<name>'s <agent>"
 */
export function delegateWho(
  values: Record<string, string>,
  memberName: (instance: string) => string | undefined,
  agentName: (agent: string) => string
): string {
  const person = memberName(values.instance ?? '') ?? values.instance ?? ''
  const agent = values.agent ?? ''
  if (!agent || agent === 'gateway') return person
  return `${person} · ${agentName(agent)}`
}

export type ApprovalClock =
  | { kind: 'expiresIn'; ms: number }
  | { kind: 'expired' }
  | { kind: 'waiting'; ms: number }

/**
 * "Expires in 12 min" while a deadline is ahead, "expired" once it's past,
 * else (no deadline) how long it has waited
 */
export function approvalClock(
  now: number,
  createdAt: number,
  expiresAt: number | null | undefined
): ApprovalClock {
  if (expiresAt === null || expiresAt === undefined) {
    return { kind: 'waiting', ms: Math.max(0, now - createdAt) }
  }
  return expiresAt <= now
    ? { kind: 'expired' }
    : { kind: 'expiresIn', ms: expiresAt - now }
}

/** Whole minutes, at least one: "12 min" */
export function minutes(ms: number): number {
  return Math.max(1, Math.round(ms / 60_000))
}
