/**
 * Language initialization utilities for detecting and applying the user's
 * preferred language at app startup.
 */
import { getSystemLocale } from '@/lib/system-locale'
import i18n, { availableLanguages } from './config'
import { logger } from '@/lib/logger'

/**
 * The available language matching a locale tag ("ja-JP", "ja_JP", "fr"),
 * or English when there is none.
 */
export function languageForLocale(systemLocale: string | null): string {
  const langCode = systemLocale?.split(/[-_]/)[0]?.toLowerCase()
  return langCode && availableLanguages.includes(langCode) ? langCode : 'en'
}

/** The language the OS (or browser, on the web) asks for */
export async function resolveSystemLanguage(): Promise<string> {
  const systemLocale = await getSystemLocale()
  logger.debug('Detected system locale', { systemLocale })
  return languageForLocale(systemLocale)
}

/**
 * Initialize the application language.
 *
 * Priority:
 * 1. User's saved language preference (if set)
 * 2. System locale (if we have translations for it)
 * 3. English (fallback)
 *
 * @param savedLanguage - The user's saved language preference from preferences
 */
export async function initializeLanguage(
  savedLanguage: string | null
): Promise<void> {
  try {
    if (savedLanguage) {
      // User has an explicit preference
      if (availableLanguages.includes(savedLanguage)) {
        await i18n.changeLanguage(savedLanguage)
        logger.info('Language set from user preference', {
          language: savedLanguage,
        })
      } else {
        logger.warn('Saved language not available, using English', {
          savedLanguage,
          availableLanguages,
        })
        await i18n.changeLanguage('en')
      }
      return
    }

    // No saved preference: follow the system locale
    const language = await resolveSystemLanguage()
    await i18n.changeLanguage(language)
    logger.info('Language set from system locale', { language })
  } catch (error) {
    logger.error('Failed to initialize language', { error })
    // Ensure we have some language set
    await i18n.changeLanguage('en')
  }
}
