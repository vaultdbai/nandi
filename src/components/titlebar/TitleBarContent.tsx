import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { useUIStore } from '@/store/ui-store'
import { executeCommand, useCommandContext } from '@/lib/commands'
import { UpdateButton } from './UpdateButton'
import { UserMenu } from './UserMenu'
import { PanelLeft, PanelLeftClose, Settings } from 'lucide-react'
import { NandiBull } from '@/assets/brand/NandiBull'
import { useMascotName, useNandiState } from '@/hooks/useMascot'

/**
 * Left-side toolbar actions (sidebar toggle).
 * Place this after window controls on macOS, or at the start on Windows/Linux.
 */
export function TitleBarLeftActions() {
  const { t } = useTranslation()
  const leftSidebarVisible = useUIStore(state => state.leftSidebarVisible)
  const toggleLeftSidebar = useUIStore(state => state.toggleLeftSidebar)
  const leftLabel = t(
    leftSidebarVisible ? 'titlebar.hideLeftSidebar' : 'titlebar.showLeftSidebar'
  )

  return (
    <div className="flex items-center gap-1">
      <Button
        onClick={toggleLeftSidebar}
        variant="ghost"
        size="icon"
        className="size-7 text-foreground/70 hover:text-foreground"
        title={leftLabel}
        aria-label={leftLabel}
      >
        {leftSidebarVisible ? (
          <PanelLeftClose className="size-3.5" />
        ) : (
          <PanelLeft className="size-3.5" />
        )}
      </Button>
    </div>
  )
}

/**
 * Right-side toolbar actions (updates, account, settings).
 * Place this before window controls on Windows, or at the end on macOS/Linux.
 */
export function TitleBarRightActions() {
  const { t } = useTranslation()
  const commandContext = useCommandContext()

  const handleOpenPreferences = async () => {
    const result = await executeCommand('open-preferences', commandContext)
    if (!result.success && result.error) {
      commandContext.showToast(result.error, 'error')
    }
  }

  return (
    <div className="flex items-center gap-1">
      <UpdateButton />
      <UserMenu />

      <Button
        onClick={handleOpenPreferences}
        variant="ghost"
        size="icon"
        className="size-7 text-foreground/70 hover:text-foreground"
        title={t('titlebar.settings')}
        aria-label={t('titlebar.settings')}
      >
        <Settings className="size-3.5" />
      </Button>
    </div>
  )
}

interface TitleBarTitleProps {
  title?: string
}

/**
 * Centered title for the title bar.
 * Uses absolute positioning to stay centered regardless of other content.
 */
export function TitleBarTitle({ title = 'Nandi' }: TitleBarTitleProps) {
  const state = useNandiState()
  const name = useMascotName()
  return (
    <div className="pointer-events-none absolute left-1/2 top-1/2 flex -translate-x-1/2 -translate-y-1/2 items-center gap-2">
      {/* The bull is the notification badge: it rises when an approval
          waits. The Inbox count says the same in words. */}
      <NandiBull state={state} name={name} size={26} />
      <span className="text-sm font-semibold tracking-tight text-foreground">
        {title}
      </span>
    </div>
  )
}

/**
 * Combined toolbar content for simple layouts.
 * Use this for Linux or when you want all toolbar items in one fragment.
 *
 * For more control, use TitleBarLeftActions, TitleBarRightActions, and TitleBarTitle separately.
 */
export function TitleBarContent({ title = 'Nandi' }: TitleBarTitleProps) {
  return (
    <>
      <TitleBarLeftActions />
      <TitleBarTitle title={title} />
      <TitleBarRightActions />
    </>
  )
}
