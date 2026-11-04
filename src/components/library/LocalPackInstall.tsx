import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { FolderOpen } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { client, isDesktop } from '@/lib/client'
import { toastError } from '@/lib/toast-error'
import { InstallConsentDialog } from './InstallConsentDialog'
import type { LibraryEntry } from './library-entry'
import { packItem } from './library-model'

/** A native picker: a folder, or one `.zip` file; `null` when cancelled */
async function pickPack(kind: 'folder' | 'zip'): Promise<string | null> {
  const { open } = await import('@tauri-apps/plugin-dialog')
  const chosen = await open(
    kind === 'folder'
      ? { directory: true, multiple: false }
      : {
          directory: false,
          multiple: false,
          filters: [{ name: 'Zip', extensions: ['zip'] }],
        }
  )
  return typeof chosen === 'string' ? chosen : null
}

/**
 * "Install from this computer": a starter pack from a folder or a `.zip`,
 * with no internet and no relay. It goes through the same consent dialog
 * and checks as a registry pack, and is marked unverified (nothing proves
 * who published it). Desktop only: the browser can't pick host paths.
 */
export function LocalPackInstall() {
  const { t } = useTranslation()
  const [picked, setPicked] = useState<{
    path: string
    entry: LibraryEntry
  } | null>(null)

  if (!isDesktop()) return null

  const choose = async (kind: 'folder' | 'zip') => {
    const path = await pickPack(kind)
    if (!path) return
    const result = await client.inspectLocalPack(path)
    if (result.status === 'error') {
      toastError(result.error, t('library.local.unreadable'))
      return
    }
    const pack = result.data
    setPicked({
      path,
      entry: { item: packItem(pack), source: { type: 'pack', pack } },
    })
  }

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button size="sm" variant="outline" data-testid="local-pack-install">
            <FolderOpen />
            {t('library.local.button')}
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={() => void choose('folder')}>
            {t('library.local.folder')}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => void choose('zip')}>
            {t('library.local.zip')}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <InstallConsentDialog
        entry={picked?.entry ?? null}
        localPath={picked?.path}
        onOpenChange={open => !open && setPicked(null)}
      />
    </>
  )
}
