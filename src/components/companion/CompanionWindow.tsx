import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { QueryClientProvider } from '@tanstack/react-query'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import '@/i18n'
import { initializeLanguage } from '@/i18n/language-init'
import { ErrorBoundary } from '@/components/ErrorBoundary'
import { useAgents } from '@/hooks/useAgents'
import { useCompanion } from '@/hooks/useCompanion'
import { useCompanionNotifications } from '@/hooks/useCompanionNotifications'
import { useHomeFeed } from '@/hooks/useHome'
import { ViewOpenerContext, type ViewOpener } from '@/hooks/useViewOpener'
import { client, isDesktop } from '@/lib/client'
import {
  COLLAPSE_EVENT,
  EXPAND_EVENT,
  FOCUS_REQUEST_EVENT,
} from '@/lib/companion-events'
import { logger } from '@/lib/logger'
import { queryClient } from '@/lib/query-client'
import { openSettings } from '@/lib/settings-nav'
import { cn } from '@/lib/utils'
import { useCompanionStore } from '@/store/companion-store'
import { useNavigationStore } from '@/store/navigation-store'
import { CompanionCollapsed } from './CompanionCollapsed'
import { CompanionExpanded, type CompanionFeed } from './CompanionExpanded'
import { CompanionOverlay } from './CompanionOverlay'
import {
  DONE_POSE_MS,
  handledOne,
  orbFor,
  poseFor,
  requestToPin,
  summaryParts,
} from './companion-model'
import {
  companionVisible,
  followTheme,
  hideCompanion,
  openInMain,
  openSettingsInMain,
  resizeCompanion,
  showBisonMenu,
  startDragging,
  type CompanionState,
} from './companion-window'

/** How often a hidden window is noticed (for notifications) */
const VISIBILITY_POLL_MS = 10_000

/**
 * The companion window (label `companion`): an orb that expands to a card
 * and a full-width overlay. Rendered by `src/companion-main.tsx` in its own
 * webview, so it sets up its own query client, theme, and language.
 */
export function CompanionWindow() {
  useEffect(() => followTheme(), [])

  useEffect(() => {
    void client
      .loadPreferences()
      .then(result =>
        initializeLanguage(result.status === 'ok' ? result.data.language : null)
      )
      .catch((error: unknown) => {
        logger.warn('Companion: language init failed', { error })
      })
  }, [])

  return (
    <ErrorBoundary>
      <QueryClientProvider client={queryClient}>
        <CompanionApp />
      </QueryClientProvider>
    </ErrorBoundary>
  )
}

/** Mod+Shift+F switches between the card and the overlay */
function isOverlayHotkey(event: KeyboardEvent): boolean {
  return (
    (event.metaKey || event.ctrlKey) &&
    event.shiftKey &&
    event.key.toLowerCase() === 'f'
  )
}

/**
 * The companion's content; needs a QueryClientProvider. Three states:
 * collapsed (an orb), expanded (a card), and overlay (full width).
 *
 * - orb click (or the hotkey bringing the window forward) → expanded
 * - expand button or Mod+Shift+F → overlay; minimize (or Escape) → expanded
 * - expanded: Escape or a click outside → collapsed
 *
 * Its lists are Home's (`useHomeFeed`), so the orb's count is the Home
 * badge's. Standalone it sizes its own window and hands links to the main
 * window; `docked` it floats in the main window's corner (the browser has
 * no companion window) and navigates there directly.
 */
export function CompanionApp({ docked = false }: { docked?: boolean }) {
  const { t } = useTranslation()
  const companion = useCompanion()
  const home = useHomeFeed()
  const { data: agents = [] } = useAgents()
  const focusedRequestId = useCompanionStore(state => state.focusedRequestId)
  const [state, setState] = useState<CompanionState>('collapsed')
  const [windowShown, setWindowShown] = useState(true)
  const containerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!docked) void resizeCompanion(state)
  }, [state, docked])

  // Watch in companion (desktop: the backend's event to this window)
  useEffect(() => {
    if (docked || !isDesktop()) return
    const unlisten = listen<{ requestId?: unknown }>(
      FOCUS_REQUEST_EVENT,
      event => {
        const id = event.payload.requestId
        if (typeof id !== 'string') return
        useCompanionStore.getState().focusRequest(id)
        setState(current => (current === 'collapsed' ? 'expanded' : current))
      }
    )
    return () => {
      void unlisten.then(fn => fn()).catch(() => undefined)
    }
  }, [docked])

  // Web: Watch in companion sets the shared store; a new focus opens the card
  const [seenFocus, setSeenFocus] = useState<string | null>(null)
  if (docked && focusedRequestId !== seenFocus) {
    setSeenFocus(focusedRequestId)
    if (focusedRequestId && state === 'collapsed') setState('expanded')
  }

  // The hotkey (or Open from the main window) asks for the card; focus
  // alone doesn't open it, so pressing the bison to drag it doesn't
  useEffect(() => {
    if (docked || !isDesktop()) return
    const window = getCurrentWindow()
    const unlistenFocus = window.onFocusChanged(({ payload }) => {
      if (payload) setWindowShown(true)
    })
    const unlistenExpand = listen(EXPAND_EVENT, () =>
      setState(current => (current === 'collapsed' ? 'expanded' : current))
    )
    const unlistenCollapse = listen(COLLAPSE_EVENT, () => setState('collapsed'))
    return () => {
      for (const unlisten of [unlistenFocus, unlistenExpand, unlistenCollapse])
        void unlisten.then(fn => fn()).catch(() => undefined)
    }
  }, [docked])

  // The bison shows `done` for a moment after you handled something
  const inboxCount = home.inbox.count
  const [seenCount, setSeenCount] = useState<number | null>(null)
  const [recentlyDone, setRecentlyDone] = useState(false)
  if (!home.isPending && inboxCount !== seenCount) {
    if (handledOne(seenCount, inboxCount)) setRecentlyDone(true)
    setSeenCount(inboxCount)
  }
  useEffect(() => {
    if (!recentlyDone) return
    const timer = setTimeout(() => setRecentlyDone(false), DONE_POSE_MS)
    return () => clearTimeout(timer)
  }, [recentlyDone])

  // A hidden window can't show what's new: notifications take over
  useEffect(() => {
    if (docked || !isDesktop()) return
    const check = () => void companionVisible().then(setWindowShown)
    check()
    const timer = setInterval(check, VISIBILITY_POLL_MS)
    return () => clearInterval(timer)
  }, [docked])

  useCompanionNotifications({
    items: home.inbox.items,
    away: state === 'collapsed' || !windowShown,
    enabled: companion.notify,
    results: companion.notifyResults,
  })

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return
      if (isOverlayHotkey(event) && state !== 'collapsed') {
        event.preventDefault()
        setState(current => (current === 'overlay' ? 'expanded' : 'overlay'))
        return
      }
      if (event.key !== 'Escape' || state === 'collapsed') return
      event.preventDefault()
      setState(state === 'overlay' ? 'expanded' : 'collapsed')
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [state])

  // A click outside collapses the card: the window losing focus when
  // standalone, a pointer down elsewhere in the page when docked
  useEffect(() => {
    if (state !== 'expanded') return
    if (!docked) {
      const onBlur = () => {
        if (!companion.pinned) setState('collapsed')
      }
      window.addEventListener('blur', onBlur)
      return () => window.removeEventListener('blur', onBlur)
    }
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node | null
      if (target && containerRef.current?.contains(target)) return
      // Menus and dialogs render in portals outside the card
      if (
        (event.target as Element | null)?.closest?.(
          '[data-radix-popper-content-wrapper], [role="dialog"], [role="alertdialog"]'
        )
      )
        return
      setState('collapsed')
    }
    document.addEventListener('pointerdown', onPointerDown)
    return () => document.removeEventListener('pointerdown', onPointerDown)
  }, [state, docked, companion.pinned])

  const opener: ViewOpener = docked
    ? {
        open: view => useNavigationStore.getState().navigate(view),
        openSettings: pane => openSettings(pane),
      }
    : {
        // A Request stays here first: pinned as a live card you can ask
        // about; its › (or opening it again) goes to the main window
        open: view => {
          const store = useCompanionStore.getState()
          const pin = requestToPin(view, store.focusedRequestId)
          if (pin) store.focusRequest(pin)
          else void openInMain(view)
        },
        openSettings: pane => void openSettingsInMain(pane),
      }
  // With the bison on the desktop, closing the card goes back to it
  const bubble = !docked && isDesktop() && (companion.snapshot?.bubble ?? true)
  const hide = () => {
    if (docked || bubble) setState('collapsed')
    else void hideCompanion()
  }

  const blocker = companion.snapshot?.blocker ?? null
  const orb = orbFor({
    inboxCount: home.inbox.count,
    running: home.running.length,
    blocker,
  })
  const counts = {
    needsYou: home.inbox.count,
    running: home.running.length,
    doneToday: companion.snapshot?.completedToday ?? 0,
  }
  const tooltip =
    blocker === 'noModel'
      ? t('companion.problem.noModel')
      : summaryParts(counts)
          .map(part =>
            t(`companion.summary.${part.key}`, { count: part.count })
          )
          .join(' · ') || t('companion.summary.quiet')

  if (state === 'collapsed') {
    return (
      <div
        ref={containerRef}
        className={cn(
          'flex items-center justify-center',
          docked ? 'fixed end-4 bottom-4 z-40' : 'h-screen w-screen'
        )}
      >
        <CompanionCollapsed
          orb={orb}
          pose={poseFor(orb, recentlyDone)}
          tooltip={tooltip}
          onExpand={() => setState('expanded')}
          onDragStart={docked || !isDesktop() ? undefined : startDragging}
          onMenu={
            docked || !isDesktop()
              ? undefined
              : () =>
                  void showBisonMenu({
                    expand: () => setState('expanded'),
                    labels: {
                      expand: t('companion.bisonMenu.open'),
                      openNandi: t('companion.openInNandi'),
                      hide: t('companion.bisonMenu.hide'),
                      quit: t('menu.quit', { appName: 'Nandi' }),
                    },
                  })
          }
        />
      </div>
    )
  }

  const feed: CompanionFeed = {
    inbox: home.inbox.items,
    running: home.running,
    recent: home.recent,
    counts,
    firstRun:
      agents.every(agent => agent.builtin) &&
      home.running.length === 0 &&
      home.recent.length === 0,
    loading: home.isPending,
    now: home.now,
  }
  const panel = {
    companion,
    feed,
    focusedRequestId,
    onClearFocus: () => useCompanionStore.getState().clearFocus(),
    onOpenInNandi: () =>
      opener.open(
        focusedRequestId
          ? { kind: 'request', requestId: focusedRequestId }
          : { kind: 'home' }
      ),
    onOpenInStudio: () =>
      opener.open({
        kind: 'studio',
        conversationId: companion.snapshot?.conversationId ?? null,
      }),
    onToggleOverlay: () =>
      setState(current => (current === 'overlay' ? 'expanded' : 'overlay')),
    onMinimize: () => setState('collapsed'),
    onHide: hide,
    // The way out once the main window is closed (it only hides)
    onQuit: docked || !isDesktop() ? undefined : () => void client.quitNandi(),
  }

  return (
    <ViewOpenerContext value={opener}>
      <div
        ref={containerRef}
        data-state={state}
        className={cn(
          'flex flex-col overflow-hidden rounded-[var(--app-corner-radius)] border border-border bg-background text-foreground shadow-lg',
          !docked && 'h-screen w-screen',
          docked &&
            (state === 'overlay'
              ? 'fixed inset-x-4 bottom-4 z-40 h-[60vh]'
              : 'fixed end-4 bottom-4 z-40 h-[min(640px,80vh)] w-[min(420px,calc(100vw-2rem))]')
        )}
      >
        {state === 'overlay' ? (
          <CompanionOverlay {...panel} />
        ) : (
          <CompanionExpanded {...panel} />
        )}
      </div>
    </ViewOpenerContext>
  )
}
