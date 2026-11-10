import { toast } from 'sonner'
import i18n from '@/i18n/config'
import {
  friendlyErrorKey,
  friendlyErrorParams,
  isRelayUnavailable,
  rawError,
} from '@/lib/friendly-error'
import { useNavigationStore } from '@/store/navigation-store'

/**
 * A failure toast in plain words (`title`, else the generic sentence for
 * the error), with "Copy details" for the raw text. Build `title` with
 * `plainError` so the raw text never shows in it.
 */
export function toastError(error: unknown, title?: string): void {
  const raw = rawError(error)
  toast.error(title ?? plainError(error), {
    action: raw
      ? {
          label: i18n.t('errors.copyDetails'),
          onClick: () => void navigator.clipboard?.writeText(raw),
        }
      : undefined,
  })
}

/** The plain sentence for an error, to put inside a screen's own message */
export function plainError(error: unknown): string {
  return i18n.t(
    friendlyErrorKey(error),
    friendlyErrorParams(error, i18n.language)
  )
}

/**
 * A person-to-person action's failure: for `RelayUnavailable`, the calm
 * "Connect to your company's relay" with a way to Network › Connection;
 * anything else goes to `otherwise`. For places with no room for
 * `RelayConnectNotice`.
 */
export function toastPersonError(
  error: unknown,
  otherwise: (error: unknown) => void
): void {
  if (!isRelayUnavailable(error)) {
    otherwise(error)
    return
  }
  toast(i18n.t('relayConnect.title'), {
    description: i18n.t('relayConnect.description'),
    action: {
      label: i18n.t('relayConnect.action'),
      onClick: () =>
        useNavigationStore
          .getState()
          .navigate({ kind: 'network', tab: 'connection' }),
    },
  })
}
