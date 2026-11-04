import { useTranslation } from 'react-i18next'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { useState } from 'react'
import { Check, Monitor, Moon, Sun } from 'lucide-react'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Switch } from '@/components/ui/switch'
import { companionQueryKeys, useCompanionSnapshot } from '@/hooks/useCompanion'
import {
  BISON_MOTIONS,
  bisonMotion,
  setBisonMotion,
  type BisonMotion,
} from '@/lib/bison-motion'
import { client, isDesktop } from '@/lib/client'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { useTheme } from '@/hooks/use-theme'
import type { Theme } from '@/lib/theme-context'
import { cn } from '@/lib/utils'
import { PALETTES, type PaletteId } from '@/themes/palettes'
import { RenameMascot } from '../RenameMascot'
import { SettingsField, SettingsSection } from '../shared/SettingsComponents'
import { usePreferences, useSavePreferences } from '@/services/preferences'
import { availableLanguages, resolveSystemLanguage } from '@/i18n'
import { logger } from '@/lib/logger'

// Language display names (native names)
const languageNames: Record<string, string> = {
  en: 'English',
  fr: 'Français',
  ar: 'العربية',
  ja: '日本語',
}

export function AppearancePane() {
  const { t, i18n } = useTranslation()
  const { theme, setTheme, palette, setPalette } = useTheme()
  const { data: preferences } = usePreferences()
  const savePreferences = useSavePreferences()

  const handlePaletteChange = (value: PaletteId) => {
    setPalette(value)
    if (preferences) {
      savePreferences.mutate({ ...preferences, palette: value })
    }
  }

  const handleThemeChange = (value: Theme) => {
    // Update the theme provider immediately for instant UI feedback
    setTheme(value)

    // Persist the theme preference to disk, preserving other preferences
    if (preferences) {
      savePreferences.mutate({ ...preferences, theme: value })
    }
  }

  const handleLanguageChange = async (value: string) => {
    const language = value === 'system' ? null : value

    try {
      // Change the language immediately for instant UI feedback
      await i18n.changeLanguage(language ?? (await resolveSystemLanguage()))
    } catch (error) {
      logger.error('Failed to change language', { error })
      toast.error(t('toast.error.generic'))
      return
    }

    // Persist the language preference to disk
    if (preferences) {
      savePreferences.mutate({ ...preferences, language })
    }
  }

  // Determine the current language value for the select
  const currentLanguageValue = preferences?.language ?? 'system'

  return (
    <div className="space-y-6">
      <SettingsSection title={t('preferences.appearance.language')}>
        <SettingsField
          label={t('preferences.appearance.language')}
          description={t('preferences.appearance.languageDescription')}
        >
          <Select
            value={currentLanguageValue}
            onValueChange={handleLanguageChange}
            disabled={savePreferences.isPending}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="system">
                {t('preferences.appearance.language.system')}
              </SelectItem>
              {availableLanguages.map(lang => (
                <SelectItem key={lang} value={lang}>
                  {languageNames[lang] ?? lang}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingsField>
      </SettingsSection>

      <SettingsSection title={t('preferences.appearance.theme')}>
        <SettingsField
          label={t('preferences.appearance.mode')}
          description={t('preferences.appearance.modeDescription')}
        >
          <ToggleGroup
            type="single"
            variant="outline"
            value={theme}
            onValueChange={value => {
              if (value) handleThemeChange(value as Theme)
            }}
            disabled={savePreferences.isPending}
            aria-label={t('preferences.appearance.mode')}
          >
            <ToggleGroupItem value="light" className="gap-1.5 px-3">
              <Sun />
              {t('preferences.appearance.theme.light')}
            </ToggleGroupItem>
            <ToggleGroupItem value="dark" className="gap-1.5 px-3">
              <Moon />
              {t('preferences.appearance.theme.dark')}
            </ToggleGroupItem>
            <ToggleGroupItem value="system" className="gap-1.5 px-3">
              <Monitor />
              {t('preferences.appearance.theme.system')}
            </ToggleGroupItem>
          </ToggleGroup>
        </SettingsField>

        <SettingsField
          label={t('preferences.appearance.palette')}
          description={t('preferences.appearance.paletteDescription')}
        >
          <div
            role="radiogroup"
            aria-label={t('preferences.appearance.palette')}
            className="grid grid-cols-2 gap-3 sm:grid-cols-3"
          >
            {PALETTES.map(option => (
              <PaletteCard
                key={option.id}
                id={option.id}
                selected={option.id === palette}
                onSelect={() => handlePaletteChange(option.id)}
              />
            ))}
          </div>
        </SettingsField>
      </SettingsSection>
      <RenameMascot />
      {isDesktop() && <CompanionSettings />}
    </div>
  )
}

/** The companion's hotkey and "Notify me" (also in the companion's ⋯ menu) */
function CompanionSettings() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const { data: snapshot } = useCompanionSnapshot()
  const notify = snapshot?.notify ?? true
  const notifyResults = snapshot?.notifyResults ?? true
  const bubble = snapshot?.bubble ?? true
  const bisonOnly = snapshot?.startBisonOnly ?? false

  const setBisonOnly = async (next: boolean) => {
    const result = await client.setCompanionStartBisonOnly(next)
    if (result.status === 'error') {
      toast.error(result.error)
      return
    }
    await queryClient.invalidateQueries({
      queryKey: companionQueryKeys.snapshot(),
    })
  }
  const [motion, setMotion] = useState<BisonMotion>(bisonMotion)

  const setBubble = async (next: boolean) => {
    const result = await client.setCompanionBubble(next)
    if (result.status === 'error') {
      toast.error(result.error)
      return
    }
    await queryClient.invalidateQueries({
      queryKey: companionQueryKeys.snapshot(),
    })
  }

  const setNotifyResults = async (next: boolean) => {
    const result = await client.setCompanionNotifyResults(next)
    if (result.status === 'error') {
      toast.error(result.error)
      return
    }
    await queryClient.invalidateQueries({
      queryKey: companionQueryKeys.snapshot(),
    })
  }

  const setNotify = async (next: boolean) => {
    const result = await client.setCompanionNotify(next)
    if (result.status === 'error') {
      toast.error(result.error)
      return
    }
    await queryClient.invalidateQueries({
      queryKey: companionQueryKeys.snapshot(),
    })
  }

  return (
    <SettingsSection title={t('preferences.appearance.companion')}>
      <SettingsField
        label={t('preferences.appearance.companionHotkey')}
        description={t('preferences.appearance.companionHotkeyDescription')}
      >
        <kbd className="w-fit rounded border bg-muted px-2 py-1 font-mono text-xs">
          {t('preferences.appearance.companionHotkeyValue')}
        </kbd>
      </SettingsField>
      <div className="flex items-center justify-between gap-4">
        <div className="space-y-0.5">
          <Label htmlFor="companion-bubble">
            {t('preferences.appearance.companionBubble')}
          </Label>
          <p className="text-sm text-muted-foreground">
            {t('preferences.appearance.companionBubbleDescription')}
          </p>
        </div>
        <Switch
          id="companion-bubble"
          checked={bubble}
          onCheckedChange={checked => void setBubble(checked)}
        />
      </div>
      <div className="flex items-center justify-between gap-4">
        <div className="space-y-0.5">
          <Label htmlFor="companion-bison-only">
            {t('preferences.appearance.startBisonOnly')}
          </Label>
          <p className="text-sm text-muted-foreground">
            {t('preferences.appearance.startBisonOnlyDescription')}
          </p>
        </div>
        <Switch
          id="companion-bison-only"
          checked={bubble && bisonOnly}
          disabled={!bubble}
          onCheckedChange={checked => void setBisonOnly(checked)}
        />
      </div>
      <div className="flex items-center justify-between gap-4">
        <div className="space-y-0.5">
          <Label htmlFor="bison-motion">
            {t('preferences.appearance.bisonMotion')}
          </Label>
          <p className="text-sm text-muted-foreground">
            {t('preferences.appearance.bisonMotionDescription')}
          </p>
        </div>
        <NativeSelect
          id="bison-motion"
          value={motion}
          onChange={event => {
            const next = BISON_MOTIONS.find(m => m === event.target.value)
            if (!next) return
            setMotion(next)
            setBisonMotion(next)
          }}
        >
          {BISON_MOTIONS.map(m => (
            <NativeSelectOption key={m} value={m}>
              {t(`preferences.appearance.bisonMotion.${m}`)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </div>
      <div className="flex items-center justify-between gap-4">
        <div className="space-y-0.5">
          <Label htmlFor="companion-notify">
            {t('preferences.appearance.companionNotify')}
          </Label>
          <p className="text-sm text-muted-foreground">
            {t('preferences.appearance.companionNotifyDescription')}
          </p>
        </div>
        <Switch
          id="companion-notify"
          checked={notify}
          onCheckedChange={checked => void setNotify(checked)}
        />
      </div>
      <div className="flex items-center justify-between gap-4">
        <div className="space-y-0.5">
          <Label htmlFor="companion-notify-results">
            {t('preferences.appearance.companionNotifyResults')}
          </Label>
          <p className="text-sm text-muted-foreground">
            {t('preferences.appearance.companionNotifyResultsDescription')}
          </p>
        </div>
        <Switch
          id="companion-notify-results"
          checked={notifyResults}
          onCheckedChange={checked => void setNotifyResults(checked)}
        />
      </div>
    </SettingsSection>
  )
}

/**
 * A palette option with a live miniature of the app (sidebar, title, a
 * card, and a button), rendered with that palette's own tokens through a
 * scoped `data-theme`.
 */
function PaletteCard({
  id,
  selected,
  onSelect,
}: {
  id: PaletteId
  selected: boolean
  onSelect: () => void
}) {
  const { t } = useTranslation()
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={onSelect}
      className={cn(
        'group flex flex-col overflow-hidden rounded-xl border bg-card text-start transition-all hover:-translate-y-0.5 hover:shadow-md focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none',
        selected && 'border-primary ring-2 ring-primary/40'
      )}
    >
      <PalettePreview id={id} />
      <div className="flex items-center gap-2 px-3 py-2">
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium">
            {t(`preferences.appearance.palette.${id}`)}
          </div>
          <div className="truncate text-xs text-muted-foreground">
            {t(`preferences.appearance.palette.${id}.description`)}
          </div>
        </div>
        {selected && (
          <span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-primary text-primary-foreground">
            <Check className="size-3" strokeWidth={3} aria-hidden />
          </span>
        )}
      </div>
    </button>
  )
}

/** The palette's tokens re-declared on a wrapper, so it previews in place */
function PalettePreview({ id }: { id: PaletteId }) {
  const option = PALETTES.find(p => p.id === id)
  const style = {
    '--brand-h': option?.hue,
    '--brand-c': option?.chroma,
  } as React.CSSProperties
  return (
    <div
      aria-hidden
      style={style}
      className="palette-preview flex h-20 border-b bg-background"
    >
      <div className="surface-brand flex w-1/4 flex-col gap-1 p-1.5">
        <span className="h-1.5 w-3/4 rounded-full bg-foreground/60" />
        <span className="h-1.5 w-full rounded-full bg-primary" />
        <span className="h-1.5 w-2/3 rounded-full bg-foreground/30" />
        <span className="h-1.5 w-3/4 rounded-full bg-foreground/30" />
      </div>
      <div className="flex flex-1 flex-col gap-1.5 p-2">
        <span className="h-2 w-1/2 rounded-full bg-foreground/70" />
        <div className="flex flex-1 gap-1.5">
          <span className="flex-1 rounded-md border bg-card" />
          <span className="flex-1 rounded-md border bg-card" />
        </div>
        <div className="flex items-center gap-1">
          <span className="h-2.5 w-8 rounded bg-primary" />
          <span className="size-2.5 rounded-full bg-success" />
          <span className="size-2.5 rounded-full bg-warning" />
          <span className="size-2.5 rounded-full bg-info" />
        </div>
      </div>
    </div>
  )
}
