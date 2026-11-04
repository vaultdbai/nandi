import { useTranslation } from 'react-i18next'
import { FormGrid, ListField, TextField, type FormProps } from './form-fields'

/** An agent's skill.md frontmatter */
export function AgentForm(props: FormProps) {
  const { t } = useTranslation()
  return (
    <FormGrid>
      <TextField {...props} path="name" label={t('studio.field.name')} />
      <TextField {...props} path="model" label={t('studio.field.model')} />
      <TextField
        {...props}
        path="description"
        label={t('studio.field.description')}
      />
      <TextField {...props} path="runtime" label={t('studio.field.runtime')} />
      <ListField
        {...props}
        path="allowed-tools"
        label={t('studio.field.allowedTools')}
      />
      <ListField {...props} path="tags" label={t('studio.field.agentTags')} />
      <ListField
        {...props}
        path="provides"
        label={t('studio.field.provides')}
      />
    </FormGrid>
  )
}
