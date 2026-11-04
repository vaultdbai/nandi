import { useTranslation } from 'react-i18next'
import {
  FormGrid,
  ListField,
  NumberField,
  TextField,
  type FormProps,
} from './form-fields'

/**
 * A policy file's frontmatter (who it applies to and the rules it enforces),
 * or NANDI.md's (`gateway`): who the gateway speaks for and its limits
 */
export function PolicyForm({
  gateway = false,
  ...props
}: FormProps & { gateway?: boolean }) {
  const { t } = useTranslation()
  if (gateway) {
    return (
      <FormGrid>
        <TextField
          {...props}
          path="display_name"
          label={t('studio.field.displayName')}
        />
        <NumberField
          {...props}
          path="policy.max_concurrent"
          label={t('studio.field.maxConcurrent')}
        />
        <NumberField
          {...props}
          path="policy.rate_limit_per_hour"
          label={t('studio.field.rateLimit')}
        />
        <ListField
          {...props}
          path="policy.require_approval"
          label={t('studio.field.requireApprovalFor')}
        />
      </FormGrid>
    )
  }
  return (
    <FormGrid>
      <TextField {...props} path="name" label={t('studio.field.name')} />
      <TextField
        {...props}
        path="description"
        label={t('studio.field.description')}
      />
      <ListField
        {...props}
        path="applies_to"
        label={t('studio.field.appliesTo')}
      />
      {/* Rules the app enforces (`domain::policy`); they only take away.
          network and files are lists of objects: edit them in the YAML */}
      <ListField
        {...props}
        path="tools.allow"
        label={t('studio.field.toolsAllow')}
      />
      <ListField
        {...props}
        path="tools.deny"
        label={t('studio.field.toolsDeny')}
      />
      <ListField
        {...props}
        path="tools.ask"
        label={t('studio.field.toolsAsk')}
      />
      <ListField
        {...props}
        path="models.allow"
        label={t('studio.field.modelsAllow')}
      />
      <ListField
        {...props}
        path="connectors.allow"
        label={t('studio.field.connectorsAllow')}
      />
      <ListField
        {...props}
        path="connectors.run_on_host"
        label={t('studio.field.connectorsOnHost')}
      />
      <ListField
        {...props}
        path="pilots.allow"
        label={t('studio.field.pilotsAllow')}
      />
    </FormGrid>
  )
}
