import { describe, it, expect, beforeEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type {
  AwsProfile,
  CredentialStatus,
  SsoLoginResult,
  SsoSetup,
} from '@/lib/bindings'
import { AwsAuthPanel } from './AwsAuthPanel'

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })

function profile(
  name: string,
  overrides: Partial<AwsProfile> = {}
): AwsProfile {
  return {
    name,
    kind: 'sso',
    region: 'us-west-2',
    ssoSession: 'corp',
    ssoStartUrl: 'https://corp.awsapps.com/start',
    ssoRegion: 'us-east-1',
    ssoAccountId: '111122223333',
    ssoRoleName: 'Bedrock',
    roleArn: null,
    sourceProfile: null,
    hasStaticKeys: false,
    isDefault: false,
    ...overrides,
  }
}

function credentials(
  overrides: Partial<CredentialStatus> = {}
): CredentialStatus {
  return {
    source: 'sso',
    valid: true,
    profile: 'work',
    region: 'us-west-2',
    accountId: '111122223333',
    arn: 'arn:aws:sts::111122223333:assumed-role/Bedrock/ada',
    expiresAt: null,
    expired: false,
    error: null,
    checkedAt: Date.now(),
    ...overrides,
  }
}

const login = (status: SsoLoginResult['status']): SsoLoginResult => ({
  loginId: 'l1',
  profile: 'work',
  status,
  verificationUri: 'https://device.sso.us-east-1.amazonaws.com/',
  userCode: 'WDDD-HRQV',
  startedAt: Date.now(),
  error: null,
})

describe('AwsAuthPanel', () => {
  beforeEach(() => {
    Object.assign(commands, {
      detectAwsCli: ok({
        installed: true,
        version: '2.28.0',
        path: '/usr/bin/aws',
        supportsSso: true,
        supportsLogin: false,
      }),
      listAwsProfiles: ok([
        profile('keys', { kind: 'staticKeys', hasStaticKeys: true }),
        profile('work', { isDefault: true }),
      ]),
      getAwsBedrockApiKeyStatus: ok({ configured: false, source: null }),
      checkAwsCredentials: ok(credentials()),
      startAwsSsoLogin: ok(login('pending')),
      pollAwsSsoLogin: ok(login('succeeded')),
    })
  })

  it('checks the default profile and shows whose credentials they are', async () => {
    render(<AwsAuthPanel />)
    expect(await screen.findByText(/2\.28\.0/)).toBeInTheDocument()
    const shown = await screen.findByTestId('aws-credentials')
    expect(shown).toHaveTextContent('Bedrock')
    expect(shown).toHaveTextContent('••••3333')
    // The full ARN and account stay behind Show
    expect(shown).not.toHaveTextContent('111122223333')
    expect(shown).not.toHaveTextContent('assumed-role')
    expect(commands.checkAwsCredentials).toHaveBeenCalledWith('work', null)
  })

  it('says when the SSO session expired', async () => {
    Object.assign(commands, {
      checkAwsCredentials: ok(
        credentials({
          valid: false,
          expired: true,
          arn: null,
          error: 'expired',
        })
      ),
    })
    render(<AwsAuthPanel />)
    expect(await screen.findByTestId('aws-credentials')).toHaveTextContent(
      /expired|harness\.aws\.expired/i
    )
  })

  it('logs in with SSO and shows the code the CLI printed', async () => {
    const user = userEvent.setup()
    render(<AwsAuthPanel />)
    await user.click(await screen.findByRole('button', { name: 'Log in' }))
    await waitFor(() =>
      expect(commands.startAwsSsoLogin).toHaveBeenCalledWith('work')
    )
    expect(await screen.findByText('WDDD-HRQV')).toBeInTheDocument()
    await waitFor(() =>
      expect(screen.getByTestId('sso-login')).toHaveTextContent(
        /logged in|harness\.aws\.sso\.succeeded/i
      )
    )
  })

  it('offers the sign-in each profile has: SSO logs in, keys are replaced', async () => {
    const user = userEvent.setup()
    render(<AwsAuthPanel />)
    await user.selectOptions(
      await screen.findByRole('combobox', { name: /profile/i }),
      'keys'
    )
    expect(
      screen.queryByRole('button', { name: 'Log in' })
    ).not.toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: 'Replace keys' })
    ).toBeInTheDocument()
  })

  it('logs a console profile in only where the CLI has aws login', async () => {
    Object.assign(commands, {
      listAwsProfiles: ok([
        profile('console', { kind: 'consoleLogin', isDefault: true }),
      ]),
    })
    const { unmount } = render(<AwsAuthPanel />)
    expect(await screen.findByText(/needs a newer AWS CLI/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Log in' })).toBeNull()
    unmount()

    Object.assign(commands, {
      detectAwsCli: ok({
        installed: true,
        version: '2.32.0',
        path: '/usr/bin/aws',
        supportsSso: true,
        supportsLogin: true,
      }),
    })
    const user = userEvent.setup()
    render(<AwsAuthPanel />)
    await user.click(await screen.findByRole('button', { name: 'Log in' }))
    await waitFor(() =>
      expect(commands.startAwsSsoLogin).toHaveBeenCalledWith('console')
    )
  })

  it('adds an access-key profile through the AWS CLI', async () => {
    Object.assign(commands, {
      saveAwsKeyProfile: ok(
        profile('build', { kind: 'staticKeys', hasStaticKeys: true })
      ),
    })
    const user = userEvent.setup()
    render(<AwsAuthPanel />)
    await user.click(
      await screen.findByRole('button', { name: 'Add access keys' })
    )
    const save = screen.getByRole('button', { name: 'Save keys' })
    expect(save).toBeDisabled()
    await user.type(screen.getByLabelText('Profile name'), 'my build')
    expect(screen.getByText(/no spaces/)).toBeInTheDocument()
    await user.clear(screen.getByLabelText('Profile name'))
    await user.type(screen.getByLabelText('Profile name'), 'build')
    await user.type(screen.getByLabelText('Access key ID'), 'AKIAEXAMPLE12345')
    await user.type(screen.getByLabelText('Secret access key'), 's3cr3t')
    await user.type(screen.getByLabelText('Region'), 'eu-west-1')
    await user.click(save)
    await waitFor(() =>
      expect(commands.saveAwsKeyProfile).toHaveBeenCalledWith({
        profile: 'build',
        accessKeyId: 'AKIAEXAMPLE12345',
        secretAccessKey: 's3cr3t',
        region: 'eu-west-1',
      })
    )
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: 'Save keys' })).toBeNull()
    )
  })

  it('sets up an SSO profile: sign in, pick the account and role, then log in', async () => {
    const setup = (overrides: Partial<SsoSetup>): SsoSetup => ({
      setupId: 's1',
      status: 'pending',
      startUrl: 'https://acme.awsapps.com/start',
      ssoRegion: 'us-east-1',
      verificationUri:
        'https://device.sso.us-east-1.amazonaws.com/?user_code=ABCD-EFGH',
      userCode: 'ABCD-EFGH',
      accounts: [],
      error: null,
      ...overrides,
    })
    Object.assign(commands, {
      startAwsSsoSetup: ok(setup({})),
      pollAwsSsoSetup: vi
        .fn()
        .mockResolvedValueOnce({ status: 'ok', data: setup({}) })
        .mockResolvedValue({
          status: 'ok',
          data: setup({
            status: 'ready',
            verificationUri: null,
            userCode: null,
            accounts: [
              {
                accountId: '111122223333',
                accountName: 'Prod',
                email: null,
                roles: ['Admin', 'ReadOnly'],
              },
            ],
          }),
        }),
      finishAwsSsoSetup: ok(profile('prod-readonly')),
      cancelAwsSsoSetup: ok(null),
      openExternal: ok(null),
    })
    const user = userEvent.setup()
    render(<AwsAuthPanel />)
    await user.click(
      await screen.findByRole('button', { name: 'Add SSO profile' })
    )
    await user.type(screen.getByLabelText('Start URL'), 'http://acme')
    expect(screen.getByText(/https:\/\/ address/)).toBeInTheDocument()
    await user.clear(screen.getByLabelText('Start URL'))
    await user.type(
      screen.getByLabelText('Start URL'),
      'https://acme.awsapps.com/start'
    )
    await user.click(screen.getByRole('button', { name: 'Sign in' }))
    await waitFor(() =>
      expect(commands.startAwsSsoSetup).toHaveBeenCalledWith({
        startUrl: 'https://acme.awsapps.com/start',
        ssoRegion: 'us-east-1',
      })
    )
    expect(await screen.findByText('ABCD-EFGH')).toBeInTheDocument()

    // Approved: pick the role; the name follows it until it's typed
    const role = await screen.findByLabelText('Role', {}, { timeout: 5000 })
    await user.selectOptions(role, 'ReadOnly')
    expect(screen.getByLabelText('Profile name')).toHaveValue('prod-readonly')
    await user.click(screen.getByRole('button', { name: 'Save and log in' }))
    await waitFor(() =>
      expect(commands.finishAwsSsoSetup).toHaveBeenCalledWith({
        setupId: 's1',
        profile: 'prod-readonly',
        accountId: '111122223333',
        roleName: 'ReadOnly',
        region: null,
      })
    )
    // Then the AWS CLI signs in for the new profile
    await waitFor(() =>
      expect(commands.startAwsSsoLogin).toHaveBeenCalledWith('prod-readonly')
    )
    expect(commands.cancelAwsSsoSetup).not.toHaveBeenCalled()
  })

  it('shows a Bedrock API key without its value', async () => {
    Object.assign(commands, {
      getAwsBedrockApiKeyStatus: ok({
        configured: true,
        source: 'environment',
      }),
    })
    render(<AwsAuthPanel />)
    expect(
      await screen.findByText(/bedrock api key|harness\.aws\.apiKey/i)
    ).toBeInTheDocument()
  })

  it('says when there are no profiles', async () => {
    Object.assign(commands, { listAwsProfiles: ok([]) })
    render(<AwsAuthPanel />)
    expect(await screen.findByText(/no aws profiles yet/i)).toBeInTheDocument()
  })
})
