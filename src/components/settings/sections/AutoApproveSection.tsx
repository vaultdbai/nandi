import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Trash2 } from 'lucide-react'
import { toast } from 'sonner'
import { RelayConnectNotice } from '@/components/network/RelayConnectNotice'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Spinner } from '@/components/ui/spinner'
import {
  useAutoApproveRules,
  useSaveAutoApproveRules,
} from '@/hooks/useAutoApproveRules'
import { useNetworkSnapshot } from '@/hooks/useNetwork'
import type { AutoApproveRule, IntakeAction } from '@/lib/bindings'
import { SettingsGroup, SettingsSectionFrame } from '../SettingsFrame'
import {
  RULE_KINDS,
  actionKey,
  actionsFor,
  addRule,
  checkNewRule,
  draftToRule,
  emptyDraft,
  removeRule,
  ruleLabel,
  toggleAction,
  toggleRuleAction,
  type RuleDraft,
} from '../auto-approve/auto-approve-model'

/**
 * Settings › Auto-approve: the rules that let a screened item through
 * without a click. Says what a rule does and what it never does, lists the
 * rules (actions as checkboxes, remove) and adds new ones.
 */
export function AutoApproveSection() {
  const { t } = useTranslation()
  const rules = useAutoApproveRules()

  return (
    <SettingsSectionFrame
      title={t('settings.section.autoApprove')}
      description={t('autoApprove.description')}
    >
      <SettingsGroup title={t('autoApprove.how.title')}>
        <ul className="list-disc space-y-1 ps-5 text-sm text-muted-foreground">
          {(
            ['screening', 'verified', 'domain', 'teammate', 'noted'] as const
          ).map(line => (
            <li key={line}>{t(`autoApprove.how.${line}`)}</li>
          ))}
        </ul>
      </SettingsGroup>
      {rules.isPending && <Spinner />}
      {rules.error && (
        <p className="text-sm text-destructive">{String(rules.error)}</p>
      )}
      {rules.data && <RulesBody rules={rules.data} />}
    </SettingsSectionFrame>
  )
}

function RulesBody({ rules }: { rules: AutoApproveRule[] }) {
  const { t } = useTranslation()
  const save = useSaveAutoApproveRules()

  const saveRules = (next: AutoApproveRule[], onSaved?: () => void) =>
    save.mutate(next, {
      onSuccess: onSaved,
      onError: error =>
        toast.error(t('autoApprove.saveFailed'), {
          description: error instanceof Error ? error.message : String(error),
        }),
    })

  return (
    <>
      <SettingsGroup title={t('autoApprove.rules')}>
        {rules.length === 0 ? (
          <p
            className="rounded-md border border-dashed p-4 text-sm text-muted-foreground"
            data-testid="auto-approve-empty"
          >
            {t('autoApprove.empty')}
          </p>
        ) : (
          <ul className="flex flex-col gap-2">
            {rules.map(rule => (
              <RuleRow
                key={rule.id}
                rule={rule}
                busy={save.isPending}
                onToggle={action =>
                  saveRules(toggleRuleAction(rules, rule.id, action))
                }
                onRemove={() => saveRules(removeRule(rules, rule.id))}
              />
            ))}
          </ul>
        )}
      </SettingsGroup>
      <AddRuleForm
        rules={rules}
        busy={save.isPending}
        onAdd={(rule, onSaved) => saveRules(addRule(rules, rule), onSaved)}
      />
    </>
  )
}

function RuleRow({
  rule,
  busy,
  onToggle,
  onRemove,
}: {
  rule: AutoApproveRule
  busy: boolean
  onToggle: (action: IntakeAction) => void
  onRemove: () => void
}) {
  const { t } = useTranslation()
  const label = ruleLabel(rule.match)
  const name = t(label.key, label.values)
  return (
    <li
      className="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-md border px-3 py-2"
      data-testid="auto-approve-rule"
    >
      <span className="min-w-0 flex-1 truncate text-sm font-medium" dir="auto">
        {name}
      </span>
      {actionsFor(rule.match.kind).map(action => (
        <label key={action} className="flex items-center gap-2 text-sm">
          <Checkbox
            checked={rule.actions.includes(action)}
            disabled={busy}
            onCheckedChange={() => onToggle(action)}
          />
          {t(actionKey(action))}
        </label>
      ))}
      <Button
        size="icon"
        variant="ghost"
        disabled={busy}
        aria-label={t('autoApprove.remove', { name })}
        onClick={onRemove}
      >
        <Trash2 />
      </Button>
    </li>
  )
}

function AddRuleForm({
  rules,
  busy,
  onAdd,
}: {
  rules: AutoApproveRule[]
  busy: boolean
  onAdd: (rule: AutoApproveRule, onSaved: () => void) => void
}) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState<RuleDraft>(emptyDraft('emailDomain'))
  const network = useNetworkSnapshot(draft.kind === 'teammate')
  const rule = draftToRule(draft)
  const problem = rule ? checkNewRule(rules, rule) : null
  const members = network.data?.members ?? []
  const noRelay =
    draft.kind === 'teammate' && network.data?.relayConnected !== true

  return (
    <SettingsGroup title={t('autoApprove.add.title')}>
      <form
        className="flex flex-col gap-3"
        onSubmit={event => {
          event.preventDefault()
          if (rule && !problem) {
            onAdd(rule, () => setDraft(emptyDraft(draft.kind)))
          }
        }}
      >
        <label className="flex flex-wrap items-center gap-2 text-sm">
          {t('autoApprove.add.kind')}
          <NativeSelect
            value={draft.kind}
            onChange={event => {
              const kind = RULE_KINDS.find(k => k === event.target.value)
              if (kind) setDraft(emptyDraft(kind))
            }}
          >
            {RULE_KINDS.map(kind => (
              <NativeSelectOption key={kind} value={kind}>
                {t(`autoApprove.kind.${kind}`)}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </label>

        {draft.kind === 'teammate' ? (
          noRelay ? (
            <RelayConnectNotice variant="inline" />
          ) : (
            <NativeSelect
              aria-label={t('autoApprove.kind.teammate')}
              value={draft.teammate?.instanceId ?? ''}
              onChange={event => {
                const member = members.find(
                  m => m.instanceId === event.target.value
                )
                setDraft({
                  ...draft,
                  teammate: member
                    ? {
                        instanceId: member.instanceId,
                        name: member.displayName,
                      }
                    : null,
                })
              }}
            >
              <NativeSelectOption value="">
                {t('autoApprove.add.pickTeammate')}
              </NativeSelectOption>
              {members.map(member => (
                <NativeSelectOption
                  key={member.instanceId}
                  value={member.instanceId}
                >
                  {member.displayName}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          )
        ) : (
          <Input
            dir="ltr"
            value={draft.value}
            aria-label={t(`autoApprove.kind.${draft.kind}`)}
            aria-invalid={problem !== null}
            placeholder={t(`autoApprove.add.placeholder.${draft.kind}`)}
            onChange={event =>
              setDraft({ ...draft, value: event.target.value })
            }
          />
        )}

        {!noRelay && (
          <div className="flex flex-wrap items-center gap-4">
            {actionsFor(draft.kind).map(action => (
              <label key={action} className="flex items-center gap-2 text-sm">
                <Checkbox
                  checked={draft.actions.includes(action)}
                  onCheckedChange={() =>
                    setDraft({
                      ...draft,
                      actions: toggleAction(draft.actions, action),
                    })
                  }
                />
                {t(actionKey(action))}
              </label>
            ))}
          </div>
        )}

        {problem && <p className="text-sm text-destructive">{t(problem)}</p>}
        {!noRelay && (
          <div>
            <Button
              type="submit"
              size="sm"
              disabled={busy || rule === null || problem !== null}
            >
              {t('autoApprove.add.action')}
            </Button>
          </div>
        )}
      </form>
    </SettingsGroup>
  )
}
