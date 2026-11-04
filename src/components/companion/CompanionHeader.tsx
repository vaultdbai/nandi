import { useTranslation } from 'react-i18next'
import {
  ExternalLink,
  Maximize2,
  Minimize2,
  Minus,
  MoreHorizontal,
  PencilRuler,
  Pin,
  PinOff,
  Power,
  Sparkles,
  X,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'

interface CompanionHeaderProps {
  expanded: boolean
  onToggleExpanded: () => void
  pinned: boolean
  onPinnedChange: (pinned: boolean) => void
  notify: boolean
  onNotifyChange: (notify: boolean) => void
  onOpenInNandi: () => void
  onOpenInStudio: () => void
  onHide: () => void
  /** Shrinks to the orb */
  onMinimize?: () => void
  /** Quits Nandi; without it there's no Quit item */
  onQuit?: () => void
}

/**
 * Title bar of the undecorated companion window: drag it to move the
 * window (`data-tauri-drag-region`), pin, open in Nandi, expand, hide. One
 * assistant: there's no agent picker ("@Agent …" asks an agent).
 */
export function CompanionHeader({
  expanded,
  onToggleExpanded,
  pinned,
  onPinnedChange,
  notify,
  onNotifyChange,
  onOpenInNandi,
  onOpenInStudio,
  onHide,
  onMinimize,
  onQuit,
}: CompanionHeaderProps) {
  const { t } = useTranslation()

  return (
    <header
      data-tauri-drag-region
      className="flex h-9 shrink-0 cursor-default items-center gap-1 border-b border-border px-2 select-none"
    >
      <span
        data-tauri-drag-region
        className="flex items-center gap-1.5 px-1 text-sm font-medium"
      >
        <span className="flex size-5 shrink-0 items-center justify-center rounded-md bg-primary text-primary-foreground">
          <Sparkles aria-hidden className="size-3" />
        </span>
        {t('companion.title')}
      </span>

      <div data-tauri-drag-region className="h-full flex-1" />

      <Button
        variant="ghost"
        size="icon-sm"
        className="size-7"
        aria-pressed={pinned}
        aria-label={pinned ? t('companion.unpin') : t('companion.pin')}
        title={pinned ? t('companion.unpin') : t('companion.pin')}
        onClick={() => onPinnedChange(!pinned)}
      >
        {pinned ? <PinOff /> : <Pin />}
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        className="size-7"
        aria-label={t('companion.openInNandi')}
        title={t('companion.openInNandi')}
        onClick={onOpenInNandi}
      >
        <ExternalLink />
      </Button>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-7"
            aria-label={t('companion.more')}
            title={t('companion.more')}
          >
            <MoreHorizontal />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-56">
          <DropdownMenuCheckboxItem
            checked={notify}
            onCheckedChange={checked => onNotifyChange(checked === true)}
          >
            {t('companion.notifyMe')}
          </DropdownMenuCheckboxItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={onOpenInStudio}>
            <PencilRuler aria-hidden />
            {t('companion.openInStudio')}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={onToggleExpanded}>
            {expanded ? <Minimize2 aria-hidden /> : <Maximize2 aria-hidden />}
            {expanded ? t('companion.collapse') : t('companion.expand')}
          </DropdownMenuItem>
          {onQuit && (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={onQuit}>
                <Power aria-hidden />
                {t('menu.quit', { appName: 'Nandi' })}
              </DropdownMenuItem>
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      {onMinimize && (
        <Button
          variant="ghost"
          size="icon-sm"
          className="size-7"
          aria-label={t('companion.minimize')}
          title={t('companion.minimize')}
          onClick={onMinimize}
        >
          <Minus />
        </Button>
      )}
      <Button
        variant="ghost"
        size="icon-sm"
        className="size-7"
        aria-label={t('companion.hide')}
        title={t('companion.hide')}
        onClick={onHide}
      >
        <X />
      </Button>
    </header>
  )
}
