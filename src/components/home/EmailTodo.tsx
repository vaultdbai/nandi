import { useTranslation } from 'react-i18next'
import { ArrowRight, Mail } from 'lucide-react'
import { ConnectMailbox } from '@/components/email/ConnectMailbox'
import {
  emailTodo,
  PROVIDER_NAMES,
} from '@/components/settings/email/email-model'
import { Button } from '@/components/ui/button'
import { useEmailStatus } from '@/hooks/useEmail'
import { openSettings } from '@/lib/settings-nav'

/**
 * Home's email to-do: with no mailbox connected (skipped at setup, or
 * disconnected later), a card to connect one so the Chief of Staff can
 * keep the notes. Gone once one is connected; then the other provider is
 * offered on one quiet line.
 */
export function EmailTodo() {
  const { t } = useTranslation()
  const status = useEmailStatus()
  if (!status.data) return null
  const todo = emailTodo(status.data.providers)
  if (todo.kind === 'none') return null

  if (todo.kind === 'another') {
    return (
      <p
        className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground"
        data-testid="home-email-another"
      >
        <Mail aria-hidden className="size-4" />
        {t('home.email.another', {
          names: todo.providers.map(p => PROVIDER_NAMES[p]).join(' / '),
        })}
        <Button
          size="sm"
          variant="ghost"
          className="h-7"
          onClick={() => openSettings('email')}
        >
          {t('home.email.settings')}
        </Button>
      </p>
    )
  }

  return (
    <section
      className="flex flex-col gap-3 rounded-lg border px-4 py-3 text-sm"
      data-testid="home-email-todo"
    >
      <div className="flex items-center gap-3">
        <Mail aria-hidden className="size-4 shrink-0 text-muted-foreground" />
        <p className="min-w-0 flex-1">{t('home.email.todo')}</p>
      </div>
      <div className="flex flex-wrap items-start gap-2 ps-7">
        {todo.providers.map(provider => (
          <ConnectMailbox key={provider} provider={provider} />
        ))}
        <Button size="sm" variant="ghost" onClick={() => openSettings('email')}>
          {t('home.email.settings')}
          <ArrowRight className="rtl:rotate-180" />
        </Button>
      </div>
    </section>
  )
}
