import ReactDOM from 'react-dom/client'
import { CompanionWindow } from './components/companion'
import './App.css'
import { isTauri } from '@tauri-apps/api/core'
import { installContextMenuGuard } from './lib/context-menu'

// No browser menu (and its OS "Share") in desktop windows
if (isTauri()) installContextMenuGuard()

// The companion window (label `companion`, see tauri.conf.json).
ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <CompanionWindow />
)
