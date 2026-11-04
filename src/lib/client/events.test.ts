import { describe, expect, it } from 'vitest'
import { CORE_EVENT_NAMES } from './events'

describe('CORE_EVENT_NAMES', () => {
  it('includes the wave 2A network events', () => {
    for (const name of [
      'relay://status',
      'relay://agent_online',
      'relay://agent_offline',
      'team://changed',
      'delegation://task_started',
      'delegation://task_completed',
      'delegation://task_failed',
    ]) {
      expect(CORE_EVENT_NAMES).toContain(name)
    }
  })

  it('includes the wave final events', () => {
    for (const name of [
      'agent://tools_loaded',
      'agent://turn_aborted',
      'agent://run_compacted',
    ]) {
      expect(CORE_EVENT_NAMES).toContain(name)
    }
  })

  it('includes the local-identity freeze events', () => {
    for (const name of [
      'remote://status_changed',
      'task://state_changed',
      'task://clarification_requested',
      'gateway://config_changed',
      'network://member_online',
      'identity://changed',
      'team://roster_changed',
      'trust://relationship_updated',
    ]) {
      expect(CORE_EVENT_NAMES).toContain(name)
    }
  })

  it('includes the wave EF events', () => {
    for (const name of [
      'network://member_joined',
      'network://member_left',
      'network://profile_updated',
      'task://reassigned',
      'workflow://manual_step_ready',
      'workflow://manual_step_completed',
      'config://repo_llm_changed',
    ]) {
      expect(CORE_EVENT_NAMES).toContain(name)
    }
  })

  it('has no GitHub or Entra events', () => {
    for (const name of CORE_EVENT_NAMES) {
      expect(name).not.toMatch(/github|entra|microsoft/i)
    }
  })

  it('has no duplicates', () => {
    expect(new Set(CORE_EVENT_NAMES).size).toBe(CORE_EVENT_NAMES.length)
  })
})
