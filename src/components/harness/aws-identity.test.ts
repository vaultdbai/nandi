import { describe, expect, it } from 'vitest'
import { arnName, maskAccount, maskedIdentity } from './aws-identity'

describe('aws identity masking', () => {
  it('keeps only the user or role name of an ARN', () => {
    expect(arnName('arn:aws:iam::440955376164:user/publisher')).toBe(
      'publisher'
    )
    expect(
      arnName('arn:aws:sts::440955376164:assumed-role/Admin/alex@acme.com')
    ).toBe('Admin')
    expect(arnName('arn:aws:iam::440955376164:root')).toBe('root')
    expect(arnName(null)).toBeNull()
  })

  it('shows the account as its last 4 digits', () => {
    expect(maskAccount('440955376164')).toBe('••••6164')
    expect(maskAccount(null)).toBeNull()
  })

  it('never includes the full ARN or account', () => {
    const line = maskedIdentity(
      {
        arn: 'arn:aws:iam::440955376164:user/publisher',
        accountId: '440955376164',
        region: 'us-east-2',
      },
      masked => `account ${masked}`
    )
    expect(line).toBe('publisher · account ••••6164 · us-east-2')
    expect(line).not.toContain('440955376164')
    expect(line).not.toContain('arn:')
  })
})
