import { describe, it, expect, beforeEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { AwsProfile } from '@/lib/bindings'
import { AwsProfileField } from './AwsProfileField'

const profile = (name: string, region: string | null): AwsProfile => ({
  name,
  kind: 'sso',
  region,
  ssoSession: null,
  ssoStartUrl: null,
  ssoRegion: null,
  ssoAccountId: null,
  ssoRoleName: null,
  roleArn: null,
  sourceProfile: null,
  hasStaticKeys: false,
  isDefault: false,
})

describe('AwsProfileField', () => {
  beforeEach(() => {
    Object.assign(commands, {
      listAwsProfiles: vi.fn().mockResolvedValue({
        status: 'ok',
        data: [profile('work', 'eu-west-1'), profile('prod', null)],
      }),
    })
  })

  it('offers the default and every profile, and reports the choice', async () => {
    const onChange = vi.fn()
    render(<AwsProfileField value="" onChange={onChange} />)
    const select = screen.getByLabelText('AWS profile')
    await screen.findByRole('option', { name: 'work (eu-west-1)' })
    expect(
      screen.getByRole('option', {
        name: 'Default: the access key, else the sign-in',
      })
    ).toBeInTheDocument()

    await userEvent.selectOptions(select, 'prod')
    expect(onChange).toHaveBeenCalledWith('prod')
  })

  it('keeps a saved profile that is no longer in the AWS config', async () => {
    render(<AwsProfileField value="old" onChange={vi.fn()} />)
    expect(
      await screen.findByRole('option', {
        name: 'old (not in your AWS config)',
      })
    ).toBeInTheDocument()
    expect(screen.getByLabelText('AWS profile')).toHaveValue('old')
  })
})
