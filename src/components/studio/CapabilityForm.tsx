import { useTranslation } from 'react-i18next'
import {
  FormGrid,
  ListField,
  NumberField,
  SelectField,
  SwitchField,
  TextField,
  type FormProps,
} from './form-fields'

/** A capability's frontmatter: identity, version, and policy */
export function CapabilityForm(props: FormProps) {
  const { t } = useTranslation()
  return (
    <FormGrid>
      <TextField {...props} path="name" label={t('studio.field.name')} />
      <TextField {...props} path="version" label={t('studio.field.version')} />
      <TextField
        {...props}
        path="description"
        label={t('studio.field.description')}
      />
      <ListField {...props} path="tags" label={t('studio.field.tags')} />
      <SelectField
        {...props}
        path="policy.visibility"
        label={t('studio.field.visibility')}
        options={['private', 'team', 'network']}
      />
      <NumberField
        {...props}
        path="policy.sla_seconds"
        label={t('studio.field.slaSeconds')}
      />
      <ListField
        {...props}
        path="required_tools"
        label={t('studio.field.requiredTools')}
      />
      <SwitchField
        {...props}
        path="policy.approval_required"
        label={t('studio.field.approvalRequired')}
      />
    </FormGrid>
  )
}
