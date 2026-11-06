import { useEffect } from 'react'
import { initializeCommandSystem } from './lib/commands'
import { buildAppMenu, setupMenuLanguageListener } from './lib/menu'
import { installContextMenuGuard } from './lib/context-menu'
import { initializeLanguage } from './i18n/language-init'
import { logger } from './lib/logger'
import { cleanupOldFiles } from './lib/recovery'
import { client, isDesktop } from './lib/client'
import { startAgentSync } from './lib/agent-sync'
import { startUpdateChecks } from './store/update-store'
import './App.css'
import { MainWindow } from './components/layout/MainWindow'
import { AppShell } from './components/layout/AppShell'
import { AuthGate } from './components/layout/AuthGate'
import { PilotConsentPrompt } from './components/agent-console/PilotConsentPrompt'
import { SandboxProgressToasts } from './components/harness/SandboxProgressToasts'
import { ThemeProvider } from './components/ThemeProvider'
import { ErrorBoundary } from './components/ErrorBoundary'
import { useSquareCornersEffect } from './hooks/useSquareCornersEffect'

function App() {
  useSquareCornersEffect()

  // Initialize command system and cleanup on app startup
  useEffect(() => {
    logger.info('🚀 Frontend application starting up')
    // Agent state: listen for events once, then load snapshots. Views only
    // read the store, so agents keep running whatever is on screen.
    const stopAgentSync = startAgentSync()

    initializeCommandSystem()
    logger.debug('Command system initialized')

    // Initialize language based on saved preference or system locale
    const initLanguageAndMenu = async () => {
      try {
        // Load preferences to get saved language
        const result = await client.loadPreferences()
        const savedLanguage =
          result.status === 'ok' ? result.data.language : null

        // Initialize language (will use system locale if no preference)
        await initializeLanguage(savedLanguage)

        // Native menu exists only in the desktop app
        if (isDesktop()) {
          await buildAppMenu()
          logger.debug('Application menu built')
          setupMenuLanguageListener()
        }
      } catch (error) {
        logger.warn('Failed to initialize language or menu', { error })
      }
    }

    initLanguageAndMenu()

    // Clean up old recovery files on startup
    cleanupOldFiles().catch(error => {
      logger.warn('Failed to cleanup old recovery files', { error })
    })

    // Example of logging with context
    logger.info('App environment', {
      isDev: import.meta.env.DEV,
      mode: import.meta.env.MODE,
    })

    // Look for a newer version at startup and every few hours; the title
    // bar shows an Update button when there is one. Desktop-only.
    const stopUpdateChecks = isDesktop() ? startUpdateChecks() : undefined
    // No browser menu (and its OS "Share") in the desktop app
    const removeMenuGuard = isDesktop() ? installContextMenuGuard() : undefined
    return () => {
      removeMenuGuard?.()
      stopUpdateChecks?.()
      stopAgentSync()
    }
  }, [])

  return (
    <ErrorBoundary>
      <ThemeProvider>
        <MainWindow>
          <AuthGate>
            <AppShell />
            <PilotConsentPrompt />
            <SandboxProgressToasts />
          </AuthGate>
        </MainWindow>
      </ThemeProvider>
    </ErrorBoundary>
  )
}

export default App
