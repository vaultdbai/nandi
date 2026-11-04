import { describe, expect, it } from 'vitest'
import {
  profileNameError,
  regionError,
  startUrlError,
  suggestedProfileName,
} from './aws-setup-model'

describe('aws-setup-model', () => {
  it('matches the backend rules for names, regions and start URLs', () => {
    expect(profileNameError(' dev-1.eu_x ')).toBeNull()
    for (const bad of ['', 'my work', 'a/b', 'a'.repeat(65)])
      expect(profileNameError(bad), bad).not.toBeNull()
    expect(regionError('')).toBeNull()
    expect(regionError('eu-west-1')).toBeNull()
    for (const bad of ['EU-WEST-1', 'useast', 'us east-1'])
      expect(regionError(bad), bad).not.toBeNull()
    expect(startUrlError('https://acme.awsapps.com/start')).toBeNull()
    for (const bad of ['http://acme.awsapps.com/start', 'acme', ''])
      expect(startUrlError(bad), bad).not.toBeNull()
  })

  it('suggests a profile name from the account and role', () => {
    const account = {
      accountId: '1',
      accountName: 'Prod Account (EU)',
      email: null,
      roles: [],
    }
    expect(suggestedProfileName(account, 'ReadOnly')).toBe(
      'prod-account-eu-readonly'
    )
  })
})
