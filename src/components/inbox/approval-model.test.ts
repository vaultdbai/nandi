import { describe, expect, it } from 'vitest'
import {
  approvalClock,
  approvalSummary,
  delegateWho,
  minutes,
} from './approval-model'

describe('approvalSummary', () => {
  it.each([
    [
      'shell',
      { cmd: 'git push origin main' },
      'shell',
      { cmd: 'git push origin main' },
    ],
    ['write_file', { path: 'notes.md' }, 'writeFile', { path: 'notes.md' }],
    ['read_file', { path: 'a.txt' }, 'readFile', { path: 'a.txt' }],
    ['list_dir', {}, 'listDir', { path: '.' }],
    [
      'delegate_to_agent',
      { capability: 'summarize' },
      'delegate',
      { capability: 'summarize' },
    ],
    [
      'nandi_save_artifact',
      { kind: 'agent', id: 'inbox-summary' },
      'saveArtifact',
      { kind: 'agent', id: 'inbox-summary' },
    ],
    [
      'nandi_save_artifact',
      { kind: 'trigger', trigger: { name: 'Every Monday' } },
      'saveArtifact',
      { kind: 'trigger', id: 'Every Monday' },
    ],
    ['continue_run', { reason: 'Spend $5?' }, 'pause', { reason: 'Spend $5?' }],
    [
      'pilot_browser__browser_hand_over',
      { site: 'x.com', summary: 'Launch post', draft: 'Hi' },
      'handOver',
      { site: 'x.com', summary: 'Launch post' },
    ],
    [
      'pilot_browser__setup',
      { reason: 'Check the order' },
      'pilotSetup',
      { pilot: 'Browser', reason: 'Check the order' },
    ],
    [
      'pilot_browser-headless__setup',
      {},
      'pilotSetup',
      { pilot: 'Browser (headless)', reason: '' },
    ],
    [
      'github__create_issue',
      {},
      'connector',
      { connector: 'github', tool: 'create_issue' },
    ],
    [
      'github__post_review',
      { url: 'acme/web#12', body: 'LGTM' },
      'postReview',
      { url: 'acme/web#12' },
    ],
    [
      'aws__batch_restart',
      { job_id: 'j-1', reason: 'OOM' },
      'batchRestart',
      { job: 'j-1' },
    ],
    [
      'aws__ecs_redeploy',
      { cluster: 'prod', service: 'api', reason: 'stuck' },
      'ecsRedeploy',
      { service: 'api', cluster: 'prod' },
    ],
    ['mystery', {}, 'other', { tool: 'mystery' }],
  ])('%s', (tool, args, key, values) => {
    expect(approvalSummary(tool, args)).toEqual({ key, values })
  })

  it('keeps the sentence to one short line', () => {
    const summary = approvalSummary('shell', {
      cmd: `echo\n${'x'.repeat(300)}`,
    })
    expect(summary.values.cmd).not.toContain('\n')
    expect(summary.values.cmd?.length).toBe(121)
  })
})

describe('approvalClock', () => {
  it('counts down to a deadline', () => {
    expect(approvalClock(1000, 0, 61_000)).toEqual({
      kind: 'expiresIn',
      ms: 60_000,
    })
  })

  it('is expired once the deadline passed', () => {
    expect(approvalClock(5000, 0, 5000)).toEqual({ kind: 'expired' })
  })

  it('says how long it waited without a deadline', () => {
    expect(approvalClock(90_000, 30_000, null)).toEqual({
      kind: 'waiting',
      ms: 60_000,
    })
  })

  it('rounds to whole minutes, at least one', () => {
    expect(minutes(10_000)).toBe(1)
    expect(minutes(12 * 60_000 + 20_000)).toBe(12)
  })
})

describe('delegation approvals', () => {
  const args = {
    capability: 'review-pr',
    target: 'i-lena/pr-reviewer',
    input: { text: 'Please review https://github.com/acme/web/pull/7' },
  }

  it('say who it goes to and what it asks', () => {
    const summary = approvalSummary('delegate_to_agent', args)
    expect(summary.key).toBe('delegateTo')
    expect(summary.values).toMatchObject({
      capability: 'review-pr',
      instance: 'i-lena',
      agent: 'pr-reviewer',
      text: 'Please review https://github.com/acme/web/pull/7',
    })
  })

  it("names the person, and their agent unless it's their gateway", () => {
    const name = (instance: string) =>
      instance === 'i-lena' ? 'Lena Fischer' : undefined
    const label = (agent: string) => agent.toUpperCase()
    const { values } = approvalSummary('delegate_to_agent', args)
    expect(delegateWho(values, name, label)).toBe('Lena Fischer · PR-REVIEWER')
    const gateway = approvalSummary('delegate_to_agent', {
      ...args,
      target: 'i-lena/gateway',
    })
    expect(delegateWho(gateway.values, name, label)).toBe('Lena Fischer')
    const stranger = approvalSummary('delegate_to_agent', {
      ...args,
      target: 'i-zed/gateway',
    })
    expect(delegateWho(stranger.values, name, label)).toBe('i-zed')
  })
})
