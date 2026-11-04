import { locale } from '@tauri-apps/plugin-os'
import { isDesktop } from '@/lib/client'

/** The OS locale on desktop, the browser's language on the web. */
export async function getSystemLocale(): Promise<string | null> {
  return isDesktop() ? await locale() : navigator.language || null
}
