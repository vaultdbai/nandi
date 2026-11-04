/** Values and rules for the git remote form (`RemoteForm`) */
import { DEFAULT_BRANCH_PREFIX } from '@/components/onboarding/identity-model'
import type { CredentialKind, RemoteCredential } from '@/lib/bindings'

export interface RemoteFormValues {
  url: string
  kind: CredentialKind
  token: string
  username: string
  password: string
  sshKeyPath: string
  branchPrefix: string
}

export const EMPTY_REMOTE_FORM: RemoteFormValues = {
  url: '',
  kind: 'none',
  token: '',
  username: '',
  password: '',
  sshKeyPath: '',
  branchPrefix: DEFAULT_BRANCH_PREFIX,
}

const REMOTE_URL_PATTERN =
  /^(?:(?:https?|ssh|git|file):\/\/\S+|[\w.-]+@[\w.-]+:\S+)$/i

export type RemoteUrlError = 'required' | 'invalid'

export function validateRemoteUrl(url: string): RemoteUrlError | null {
  const trimmed = url.trim()
  if (!trimmed) return 'required'
  if (!REMOTE_URL_PATTERN.test(trimmed)) return 'invalid'
  return null
}

/** The credential to send (`null` for none, or when its fields are empty) */
export function toCredential(
  values: RemoteFormValues
): RemoteCredential | null {
  switch (values.kind) {
    case 'none':
      return null
    case 'github':
      return { kind: 'github', token: null, username: null, sshKeyPath: null }
    case 'token':
      return values.token
        ? {
            kind: 'token',
            token: values.token,
            username: null,
            sshKeyPath: null,
          }
        : null
    case 'sshKey':
      return values.sshKeyPath.trim()
        ? {
            kind: 'sshKey',
            token: null,
            username: null,
            sshKeyPath: values.sshKeyPath.trim(),
          }
        : null
    case 'userPass':
      return values.username.trim()
        ? {
            kind: 'userPass',
            token: values.password,
            username: values.username.trim(),
            sshKeyPath: null,
          }
        : null
  }
}
