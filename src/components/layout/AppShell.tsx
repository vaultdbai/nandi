import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from '@/components/ui/resizable'
import { cn } from '@/lib/utils'
import { useUIStore } from '@/store/ui-store'
import { LeftSideBar } from './LeftSideBar'
import { MainPane } from './MainPane'
import { Sidebar } from './Sidebar'

/**
 * Panel sizes as percentages (react-resizable-panels v3 has no pixel units).
 * At 1280px: sidebar ≈ 260px.
 */
const LAYOUT = {
  sidebar: { default: 20, min: 14, max: 32 },
  main: { min: 35 },
} as const

/** Two panes: navigation and the current view. */
export function AppShell() {
  const leftSidebarVisible = useUIStore(state => state.leftSidebarVisible)

  return (
    <ResizablePanelGroup direction="horizontal">
      <ResizablePanel
        id="sidebar"
        order={1}
        defaultSize={LAYOUT.sidebar.default}
        minSize={LAYOUT.sidebar.min}
        maxSize={LAYOUT.sidebar.max}
        className={cn(!leftSidebarVisible && 'hidden')}
      >
        <LeftSideBar className="surface-brand border-e-0">
          <Sidebar />
        </LeftSideBar>
      </ResizablePanel>

      <ResizableHandle className={cn(!leftSidebarVisible && 'hidden')} />

      <ResizablePanel id="main" order={2} minSize={LAYOUT.main.min}>
        <MainPane />
      </ResizablePanel>
    </ResizablePanelGroup>
  )
}
