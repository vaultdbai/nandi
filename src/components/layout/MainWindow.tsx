import { TitleBar } from '@/components/titlebar/TitleBar'
import { CommandPalette } from '@/components/palette'
import { Toaster } from 'sonner'
import { useTheme } from '@/hooks/use-theme'
import { useMainWindowEventListeners } from '@/hooks/useMainWindowEventListeners'
import { CompanionApp } from '@/components/companion'
import { client } from '@/lib/client'

interface MainWindowProps {
  /** Page content rendered below the title bar */
  children: React.ReactNode
}

/**
 * App chrome: title bar, page content, and global overlays.
 * Pages (e.g. AgentConsole) own their own panel layout.
 */
export function MainWindow({ children }: MainWindowProps) {
  const { theme } = useTheme()

  // Set up global event listeners (keyboard shortcuts, etc.)
  useMainWindowEventListeners()

  return (
    <div className="flex h-screen w-full flex-col overflow-hidden rounded-[var(--app-corner-radius)] bg-background">
      <TitleBar />

      <div className="flex flex-1 overflow-hidden">{children}</div>

      {/* Global UI Components (hidden until triggered) */}
      <CommandPalette />
      {/* The browser has no companion window: it docks in the corner */}
      {client.transport === 'web' && <CompanionApp docked />}
      <Toaster
        position="bottom-right"
        theme={
          theme === 'dark' ? 'dark' : theme === 'light' ? 'light' : 'system'
        }
        className="toaster group"
        toastOptions={{
          classNames: {
            toast:
              'group toast group-[.toaster]:bg-background group-[.toaster]:text-foreground group-[.toaster]:border-border group-[.toaster]:shadow-lg',
            description: 'group-[.toast]:text-muted-foreground',
            actionButton:
              'group-[.toast]:bg-primary group-[.toast]:text-primary-foreground',
            cancelButton:
              'group-[.toast]:bg-muted group-[.toast]:text-muted-foreground',
          },
        }}
      />
    </div>
  )
}
