import { useTranslation } from 'react-i18next'

/** What Nandi reads and never does, wherever a mailbox can be connected */
export function EmailReads() {
  const { t } = useTranslation()
  return (
    <div className="rounded-md border p-4 text-sm">
      <p className="font-medium">{t('email.reads.title')}</p>
      <ul className="mt-2 list-disc space-y-1 ps-5 text-muted-foreground">
        <li>{t('email.reads.what')}</li>
        <li>{t('email.reads.never')}</li>
        <li>{t('email.reads.kept')}</li>
        <li>{t('email.reads.cited')}</li>
      </ul>
    </div>
  )
}
