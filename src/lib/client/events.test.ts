import { describe, expect, it } from 'vitest'
import bindingsSource from '@/lib/bindings.ts?raw'
import { CORE_EVENT_NAMES } from './events'

/** The wire names the generated events object maps, e.g. knowmeChanged: "knowme://changed" */
const GENERATED_EVENT_NAMES = [
  ...bindingsSource.matchAll(/^\w+: "([a-z]+:\/\/[a-z_]+)",?$/gm),
].map(match => match[1])

/** Tauri refuses to emit any other name, and the event is lost */
const TAURI_EVENT_NAME = /^[A-Za-z0-9\-/:_]+$/

describe('CORE_EVENT_NAMES', () => {
  it('includes the wave 2A network events', () => {
    for (const name of [
      'relay://status',
      'relay://agent_online',
      'relay://agent_offline',
      'relay://seal_refused',
      'network://member_key_changed',
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

  it('includes the live UI event', () => {
    expect(CORE_EVENT_NAMES).toContain('live://changed')
    expect(CORE_EVENT_NAMES).not.toContain('notes://changed')
  })

  it('are all names Tauri can emit', () => {
    const bad = CORE_EVENT_NAMES.filter(name => !TAURI_EVENT_NAME.test(name))
    expect(bad).toEqual([])
    expect(TAURI_EVENT_NAME.test('repo://agent.created')).toBe(false)
  })

  it('subscribes to every event the bindings declare', () => {
    expect(GENERATED_EVENT_NAMES.length).toBeGreaterThan(50)
    expect(
      GENERATED_EVENT_NAMES.filter(n => !CORE_EVENT_NAMES.includes(n as never))
    ).toEqual([])
  })

  it('has no duplicates', () => {
    expect(new Set(CORE_EVENT_NAMES).size).toBe(CORE_EVENT_NAMES.length)
  })
})
