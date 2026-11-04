import { useEffect, useId, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Search } from 'lucide-react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/dialog'
import { Kbd } from '@/components/ui/kbd'
import { Spinner } from '@/components/ui/spinner'
import { usePalette } from '@/hooks/usePalette'
import { useUIStore } from '@/store/ui-store'
import { PaletteResult } from './PaletteResult'
import { handlePhrase } from '@/hooks/useMascot'

/**
 * Global palette (Cmd/Ctrl+K, see `use-keyboard-shortcuts`): searches
 * agents, workflows, runs, packages, and tools; `>` switches to actions.
 * Empty input shows recent items and suggestions.
 */
export function CommandPalette() {
  const { t } = useTranslation()
  const open = useUIStore(state => state.commandPaletteOpen)

  const onOpenChange = (next: boolean) =>
    useUIStore.getState().setCommandPaletteOpen(next)

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        showCloseButton={false}
        className="top-[12%] flex max-h-[480px] translate-y-0 flex-col gap-0 overflow-hidden p-0 sm:max-w-[640px]"
      >
        <DialogTitle className="sr-only">{t('palette.title')}</DialogTitle>
        <DialogDescription className="sr-only">
          {t('palette.description')}
        </DialogDescription>
        {/* Mounted only while open, so each opening starts empty */}
        {open && <PaletteBody onClose={() => onOpenChange(false)} />}
      </DialogContent>
    </Dialog>
  )
}

function PaletteBody({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation()
  const palette = usePalette(true)
  const [active, setActive] = useState(0)
  const listId = useId()
  const listRef = useRef<HTMLDivElement>(null)
  const { items } = palette
  const activeIndex = Math.min(active, Math.max(items.length - 1, 0))
  const optionId = (index: number) => `${listId}-option-${index}`

  // Keep the highlighted row visible while arrowing through the list
  useEffect(() => {
    const el = listRef.current?.querySelector('[data-active]')
    el?.scrollIntoView?.({ block: 'nearest' })
  }, [activeIndex])

  const run = (index: number) => {
    const item = items[index]
    if (!item) return
    onClose()
    void palette.execute(item)
  }

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    // The hidden phrase is answered before any command runs
    if (e.key === 'Enter' && handlePhrase(palette.query)) {
      e.preventDefault()
      onClose()
      return
    }
    if (items.length === 0) return
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault()
        setActive((activeIndex + 1) % items.length)
        break
      case 'ArrowUp':
        e.preventDefault()
        setActive((activeIndex - 1 + items.length) % items.length)
        break
      case 'Home':
        e.preventDefault()
        setActive(0)
        break
      case 'End':
        e.preventDefault()
        setActive(items.length - 1)
        break
      case 'Enter':
        e.preventDefault()
        run(activeIndex)
        break
    }
  }

  // Each group's first row in the flat `items` list
  const offsets = palette.groups.map((_, g) =>
    palette.groups
      .slice(0, g)
      .reduce((count, group) => count + group.items.length, 0)
  )

  return (
    <>
      <div className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
        <Search aria-hidden className="size-4 shrink-0 text-muted-foreground" />
        <input
          autoFocus
          role="combobox"
          aria-expanded
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={
            items.length > 0 ? optionId(activeIndex) : undefined
          }
          aria-label={t('palette.title')}
          value={palette.query}
          onChange={e => {
            palette.setQuery(e.target.value)
            setActive(0)
          }}
          onKeyDown={onKeyDown}
          placeholder={
            palette.commandMode
              ? t('palette.commandPlaceholder')
              : t('palette.placeholder')
          }
          className="h-full min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
        />
        {palette.isLoading && <Spinner className="size-4" />}
        {palette.commandMode ? (
          <span className="shrink-0 rounded bg-primary/10 px-1.5 py-0.5 text-xs font-medium text-primary">
            {t('palette.commandMode')}
          </span>
        ) : (
          <span className="flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
            <Kbd>&gt;</Kbd>
            {t('palette.commandHint')}
          </span>
        )}
      </div>

      <div
        ref={listRef}
        id={listId}
        role="listbox"
        aria-label={t('palette.results')}
        className="min-h-0 flex-1 overflow-y-auto p-2"
      >
        {palette.groups.length === 0 ? (
          <p className="px-3 py-6 text-center text-sm text-muted-foreground">
            {palette.isLoading
              ? t('palette.searching')
              : t('palette.noResults')}
          </p>
        ) : (
          palette.groups.map((group, g) => (
            <div
              key={group.id}
              role="group"
              aria-label={t(`palette.group.${group.id}`)}
            >
              <p
                aria-hidden
                className="px-3 pt-2 pb-1 text-xs font-medium text-muted-foreground"
              >
                {t(`palette.group.${group.id}`)}
              </p>
              {group.items.map((item, i) => {
                const itemIndex = (offsets[g] ?? 0) + i
                return (
                  <PaletteResult
                    key={`${group.id}-${item.key}`}
                    id={optionId(itemIndex)}
                    item={item}
                    active={itemIndex === activeIndex}
                    onHover={() => setActive(itemIndex)}
                    onSelect={() => run(itemIndex)}
                  />
                )
              })}
            </div>
          ))
        )}
      </div>

      <div className="flex shrink-0 items-center gap-4 border-t px-3 py-2 text-xs text-muted-foreground">
        <span className="flex items-center gap-1">
          <Kbd>↑</Kbd>
          <Kbd>↓</Kbd>
          {t('palette.footer.navigate')}
        </span>
        <span className="flex items-center gap-1">
          <Kbd>↵</Kbd>
          {t('palette.footer.open')}
        </span>
        <span className="flex items-center gap-1">
          <Kbd>Esc</Kbd>
          {t('palette.footer.close')}
        </span>
      </div>
    </>
  )
}
