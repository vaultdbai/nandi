import { useTranslation } from 'react-i18next'
import { CloudUpload, UserRound } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { openSettings } from '@/lib/settings-nav'

/**
 * Account menu: Settings › Account and Settings › Backup. Identity is
 * local (no sign-in), so there's nothing to log out of.
 */
export function UserMenu() {
  const { t } = useTranslation()

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          className="h-7 min-w-7 gap-1 px-2 text-xs text-foreground/70 hover:text-foreground"
          aria-label={t('userMenu.label')}
        >
          <UserRound className="size-3.5" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-48">
        <DropdownMenuItem onSelect={() => openSettings('account')}>
          <UserRound />
          {t('settings.section.account')}
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => openSettings('backup')}>
          <CloudUpload />
          {t('settings.section.backup')}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
