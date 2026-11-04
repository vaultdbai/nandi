import {
  CalendarDays,
  Clock,
  Cpu,
  FolderOpen,
  Webhook,
  Zap,
} from 'lucide-react'
import type { TriggerSourceType } from '@/lib/tauri-bindings'

/** One icon per trigger source, used wherever triggers are listed */
export const SOURCE_ICONS: Record<TriggerSourceType, React.ReactNode> = {
  cron: <Clock />,
  calendar: <CalendarDays />,
  file: <FolderOpen />,
  webhook: <Webhook />,
  device: <Cpu />,
  event: <Zap />,
}
