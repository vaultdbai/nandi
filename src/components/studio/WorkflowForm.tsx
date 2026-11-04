import { useTranslation } from 'react-i18next'
import { FormGrid, TextField, type FormProps } from './form-fields'

/** A workflow's name; its steps are edited in the raw YAML below */
export function WorkflowForm(props: FormProps) {
  const { t } = useTranslation()
  const steps = props.text.match(/^\s*- id:/gm)?.length ?? 0
  return (
    <div className="flex flex-col gap-2">
      <FormGrid>
        <TextField {...props} path="name" label={t('studio.field.name')} />
      </FormGrid>
      <p className="text-xs text-muted-foreground">
        {t('studio.workflowSteps', { count: steps })}
      </p>
    </div>
  )
}
