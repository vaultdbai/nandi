import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Plus, TriangleAlert, X } from 'lucide-react'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { isValidHost } from '@/hooks/useAgentEditor'
import type { HostRule } from '@/lib/tauri-bindings'
import {
  ANY_SITE,
  accessMode,
  normalizeSite,
  type AccessMode,
} from './network-access-model'

const MODES: AccessMode[] = ['none', 'some', 'any']

/**
 * The sites the agent may reach: none, a list (added, changed and removed
 * here), or any site. Changes go into the Settings draft and are saved
 * with it. Methods a rule is limited to are kept on save and shown here.
 */
export function NetworkAccess({
  sites,
  rules,
  onChange,
}: {
  sites: string[]
  /** The saved rules, for their method limits */
  rules: HostRule[]
  onChange: (sites: string[]) => void
}) {
  const { t } = useTranslation()
  const [mode, setMode] = useState<AccessMode>(() => accessMode(sites))
  // Kept while "Any site" is chosen, so switching back brings it back
  const [list, setList] = useState(() =>
    sites.filter(site => site.trim() !== ANY_SITE)
  )
  const [adding, setAdding] = useState('')
  const [addError, setAddError] = useState<string | null>(null)

  const choose = (next: AccessMode) => {
    setMode(next)
    onChange(next === 'any' ? [ANY_SITE] : next === 'some' ? list : [])
  }

  const edit = (next: string[]) => {
    setList(next)
    onChange(next)
  }

  const add = () => {
    const site = normalizeSite(adding)
    if (!site) return
    const problem = !isValidHost(site)
      ? 'agentPage.network.invalid'
      : list.map(normalizeSite).includes(site)
        ? 'agentPage.network.duplicate'
        : null
    if (problem) {
      setAddError(t(problem))
      return
    }
    edit([...list, site])
    setAdding('')
    setAddError(null)
  }

  const methods = (site: string) =>
    rules.find(rule => rule.host === normalizeSite(site))?.methods ?? []

  return (
    <div className="flex flex-col gap-3">
      <RadioGroup
        value={mode}
        onValueChange={value => choose(value as AccessMode)}
        className="flex flex-wrap gap-4"
        aria-label={t('agentPage.network.title')}
      >
        {MODES.map(option => (
          <div key={option} className="flex items-center gap-2">
            <RadioGroupItem id={`network-${option}`} value={option} />
            <Label htmlFor={`network-${option}`}>
              {t(`agentPage.network.mode.${option}`)}
            </Label>
          </div>
        ))}
      </RadioGroup>

      {mode === 'none' && (
        <p className="text-xs text-muted-foreground">
          {t('agentPage.network.noneHint')}
        </p>
      )}

      {mode === 'any' && (
        <Alert>
          <TriangleAlert />
          <AlertDescription>
            {t('agentPage.network.anyWarning')}
          </AlertDescription>
        </Alert>
      )}

      {mode === 'some' && (
        <div className="flex flex-col gap-2">
          {list.length === 0 && (
            <p className="text-xs text-muted-foreground">
              {t('agentPage.network.empty')}
            </p>
          )}
          {list.length > 0 && (
            <ul className="flex flex-col gap-2">
              {list.map((site, index) => {
                const limited = methods(site)
                const invalid = site.trim() !== '' && !isValidHost(site)
                return (
                  <li key={index} className="flex flex-col gap-1">
                    <div className="flex items-center gap-2">
                      <Input
                        dir="ltr"
                        className="font-mono text-xs"
                        aria-label={t('agentPage.network.site', {
                          number: index + 1,
                        })}
                        aria-invalid={invalid}
                        value={site}
                        onChange={event =>
                          edit(
                            list.map((s, i) =>
                              i === index ? event.target.value : s
                            )
                          )
                        }
                      />
                      {limited.length > 0 && (
                        <Badge variant="outline" className="shrink-0">
                          {t('agentPage.network.methodsOnly', {
                            methods: limited.join(', '),
                          })}
                        </Badge>
                      )}
                      <Button
                        type="button"
                        size="icon"
                        variant="ghost"
                        aria-label={t('agentPage.network.remove', {
                          site: site || index + 1,
                        })}
                        onClick={() => edit(list.filter((_, i) => i !== index))}
                      >
                        <X />
                      </Button>
                    </div>
                    {invalid && (
                      <p role="alert" className="text-xs text-destructive">
                        {t('agentPage.network.invalid')}
                      </p>
                    )}
                  </li>
                )
              })}
            </ul>
          )}

          <form
            className="flex items-center gap-2"
            onSubmit={event => {
              event.preventDefault()
              add()
            }}
          >
            <Input
              dir="ltr"
              className="font-mono text-xs"
              aria-label={t('agentPage.network.addLabel')}
              placeholder={t('agentPage.network.addPlaceholder')}
              value={adding}
              onChange={event => {
                setAdding(event.target.value)
                setAddError(null)
              }}
            />
            <Button
              type="submit"
              size="sm"
              variant="outline"
              disabled={!adding.trim()}
            >
              <Plus />
              {t('agentPage.network.add')}
            </Button>
          </form>
          {addError && (
            <p role="alert" className="text-xs text-destructive">
              {addError}
            </p>
          )}
          <p className="text-xs text-muted-foreground">
            {t('agentPage.network.someHint')}
          </p>
        </div>
      )}
    </div>
  )
}
