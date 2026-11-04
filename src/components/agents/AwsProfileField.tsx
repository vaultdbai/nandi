import { useTranslation } from 'react-i18next'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { useAwsProfiles } from '@/hooks/useAwsAuth'
import { openSettings } from '@/lib/settings-nav'

/**
 * The AWS profile an agent's AWS calls run as (`aws.profile`). Empty:
 * the access key in Settings → Cloud credentials, else the sign-in profile
 * there. A saved profile missing from the user's AWS config stays listed,
 * so saving never drops it silently.
 */
export function AwsProfileField({
  value,
  onChange,
}: {
  value: string
  onChange: (profile: string) => void
}) {
  const { t } = useTranslation()
  const profiles = useAwsProfiles()
  const names = (profiles.data ?? []).map(profile => profile.name)
  // Listed while profiles load too, so the select never shows another one
  const unlisted = value !== '' && !names.includes(value)

  return (
    <div className="space-y-1.5">
      <Label htmlFor="agent-aws-profile">
        {t('agentPage.settings.awsProfile')}
      </Label>
      <NativeSelect
        id="agent-aws-profile"
        value={value}
        onChange={event => onChange(event.target.value)}
      >
        <NativeSelectOption value="">
          {t('agentPage.settings.awsProfileDefault')}
        </NativeSelectOption>
        {unlisted && (
          <NativeSelectOption value={value}>
            {profiles.isSuccess
              ? t('agentPage.settings.awsProfileMissing', { profile: value })
              : value}
          </NativeSelectOption>
        )}
        {(profiles.data ?? []).map(profile => (
          <NativeSelectOption key={profile.name} value={profile.name}>
            {profile.name}
            {profile.region ? ` (${profile.region})` : ''}
          </NativeSelectOption>
        ))}
      </NativeSelect>
      <p className="text-xs text-muted-foreground">
        {t('agentPage.settings.awsProfileHint')}{' '}
        <button
          type="button"
          className="underline underline-offset-2"
          onClick={() => openSettings('cloud')}
        >
          {t('agentPage.settings.awsProfileSettings')}
        </button>
      </p>
    </div>
  )
}
