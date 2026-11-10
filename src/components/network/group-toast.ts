import { toast } from 'sonner'
import { errorKind, rawError } from '@/lib/friendly-error'
import { plainError, toastPersonError } from '@/lib/toast-error'

/**
 * A group call failed: "Connect to your relay" when there is none; past a
 * public relay's messages a day, calmly, with when it resets; else
 * `title` with the relay's reason under it
 */
export function toastGroupError(title: string, error: unknown): void {
  toastPersonError(error, failure =>
    errorKind(failure) === 'dailyLimit'
      ? toast(title, { description: plainError(failure) })
      : toast.error(title, { description: rawError(failure) })
  )
}
