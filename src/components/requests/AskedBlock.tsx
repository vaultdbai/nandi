import { Fragment } from 'react'
import { useTranslation } from 'react-i18next'
import type { Request } from '@/lib/bindings'
import { askOf } from './request-model'

/**
 * What was asked, word for word, and by whom: the text, then every other
 * input as sent (due date, links, parameters). On a Request's page and on
 * the Run Detail of a run that served one.
 */
export function AskedBlock({ request }: { request: Request }) {
  const { t, i18n } = useTranslation()
  const ask = askOf(request.input)
  const name = request.counterpart ?? t('request.someone')
  const fields = [
    ...(request.capability
      ? [{ key: t('request.details.capability'), value: request.capability }]
      : []),
    ...ask.fields.map(field => ({
      key: field.key ? field.key.replace(/_/g, ' ') : t('request.asked.input'),
      value: field.value,
    })),
    ...(request.deadline
      ? [
          {
            key: t('request.asked.deadline'),
            value: new Date(request.deadline).toLocaleString(i18n.language),
          },
        ]
      : []),
  ]
  if (ask.text === null && fields.length === 0) return null

  return (
    <section
      aria-label={t('request.asked.title')}
      className="flex flex-col gap-2 rounded-lg border p-4"
    >
      <h2 className="text-sm font-semibold">
        {t(`request.asked.${request.direction}`, { name })}
      </h2>
      {ask.text !== null && (
        <blockquote className="max-h-72 overflow-y-auto border-s-2 ps-3 text-sm whitespace-pre-wrap select-text">
          {ask.text}
        </blockquote>
      )}
      {fields.length > 0 && (
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
          {fields.map(field => (
            <Fragment key={field.key}>
              <dt className="text-muted-foreground first-letter:uppercase">
                {field.key}
              </dt>
              <dd className="min-w-0 break-words select-text">{field.value}</dd>
            </Fragment>
          ))}
        </dl>
      )}
    </section>
  )
}
