import { describe, expect, it } from 'vitest'
import { joinMessage } from './join-model'

const URL = 'wss://relay.acme.com/relay'

describe('joinMessage', () => {
  it('names the relay joined', () => {
    expect(joinMessage({ kind: 'joined', relay: URL }, URL)).toMatchObject({
      key: 'relay.join.joined',
      params: { host: 'relay.acme.com' },
      ok: true,
    })
  })

  it("keeps the relay's own words for a full relay, verbatim", () => {
    const message =
      'This relay is at its free limit of 20 users. Add a licence file from VaultDB.ai (licensing@vaultdb.ai) to add more.'
    expect(joinMessage({ kind: 'relayFull', message }, URL)).toMatchObject({
      key: 'relay.join.relayFull',
      detail: message,
      ok: false,
    })
  })

  it('has one calm line for every other outcome', () => {
    const cases = [
      [{ kind: 'invalidUrl' }, 'relay.join.invalidUrl', null],
      [
        { kind: 'unreachable', detail: 'refused' },
        'relay.join.unreachable',
        'refused',
      ],
      [{ kind: 'codeNeeded' }, 'relay.join.codeNeeded', null],
      [{ kind: 'wrongCode' }, 'relay.join.wrongCode', null],
      [
        { kind: 'alreadyEnrolled', message: 'taken' },
        'relay.join.alreadyEnrolled',
        'taken',
      ],
      [{ kind: 'removed', message: 'gone' }, 'relay.join.removed', 'gone'],
      [{ kind: 'noEnrollment' }, 'relay.join.noEnrollment', null],
      [{ kind: 'refused', message: 'no' }, 'relay.join.refused', 'no'],
    ] as const
    for (const [outcome, key, detail] of cases) {
      expect(joinMessage(outcome, URL)).toMatchObject({
        key,
        detail,
        ok: false,
      })
    }
    expect(
      joinMessage({ kind: 'versionMismatch', relay: 3, app: 2 }, URL).params
    ).toEqual({ host: 'relay.acme.com', relay: 3, app: 2 })
  })
})
