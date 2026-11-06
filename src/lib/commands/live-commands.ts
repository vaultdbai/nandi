import { RefreshCw } from 'lucide-react'
import { refreshFromDisk } from '@/lib/live-changes'
import type { AppCommand } from './types'

/** The id F5 and Ctrl/Cmd+R run */
export const REFRESH_COMMAND = 'refresh-from-disk'

export const liveCommands: AppCommand[] = [
  {
    id: REFRESH_COMMAND,
    labelKey: 'commands.refreshFromDisk.label',
    descriptionKey: 'commands.refreshFromDisk.description',
    icon: RefreshCw,
    shortcut: 'F5',
    keywords: ['refresh', 'reload', 'disk', 'files', 'sync'],

    execute: async context => {
      try {
        await refreshFromDisk()
      } catch (error) {
        context.showToast(
          error instanceof Error ? error.message : String(error),
          'error'
        )
      }
    },
  },
]
