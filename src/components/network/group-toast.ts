import { toast } from 'sonner'
import { rawError } from '@/lib/friendly-error'
import { toastPersonError } from '@/lib/toast-error'

/**
 * A group call failed: "Connect to your relay" when there is none, else
 * `title` with the relay's reason under it
 */
export function toastGroupError(title: string, error: unknown): void {
  toastPersonError(error, failure =>
    toast.error(title, { description: rawError(failure) })
  )
}
