import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { Spinner } from '@/components/ui/spinner'
import { agentsQueryKeys, useAgents } from '@/hooks/useAgents'
import { sendToStudio } from '@/lib/agent-actions'
import { client } from '@/lib/client'
import { useNavigationStore } from '@/store/navigation-store'
import {
  AGENT_TEMPLATES,
  agentIdFrom,
  type AgentTemplate,
} from './agent-page-model'

/**
 * "New agent": a name and what it does, from a template (created here with
 * the default model, then opened), or described to Studio, which drafts
 * the whole thing.
 */
export function NewAgentDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const { data: agents } = useAgents()
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [template, setTemplate] = useState<AgentTemplate>('blank')
  const [creating, setCreating] = useState(false)

  const reset = () => {
    setName('')
    setDescription('')
    setTemplate('blank')
  }

  const create = async () => {
    setCreating(true)
    const id = agentIdFrom(
      name,
      (agents ?? []).map(agent => agent.id)
    )
    const result = await client.createAgent({
      id,
      name: name.trim(),
      description: description.trim() || null,
      model: null,
      instructions:
        template === 'blank'
          ? null
          : t(`agentPage.new.templates.${template}.instructions`),
    })
    setCreating(false)
    if (result.status === 'error') {
      toast.error(t('agentPage.new.failed'), { description: result.error })
      return
    }
    await queryClient.invalidateQueries({ queryKey: agentsQueryKeys.all })
    toast.success(t('agentPage.new.created', { name: result.data.name }))
    reset()
    onOpenChange(false)
    useNavigationStore
      .getState()
      .navigate({ kind: 'agent', agentId: result.data.id })
  }

  const describe = () => {
    const text = description.trim() || name.trim()
    onOpenChange(false)
    if (text) {
      void sendToStudio(
        t('agentPage.new.studioPrompt', { what: text }),
        [],
        null
      )
    } else {
      useNavigationStore
        .getState()
        .navigate({ kind: 'studio', conversationId: null })
    }
    reset()
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t('agentPage.new.title')}</DialogTitle>
          <DialogDescription>
            {t('agentPage.new.description')}
          </DialogDescription>
        </DialogHeader>
        <form
          className="flex flex-col gap-4"
          onSubmit={event => {
            event.preventDefault()
            if (name.trim()) void create()
          }}
        >
          <div className="space-y-1.5">
            <Label htmlFor="new-agent-name">{t('agentPage.new.name')}</Label>
            <Input
              id="new-agent-name"
              value={name}
              autoFocus
              maxLength={200}
              placeholder={t('agentPage.new.namePlaceholder')}
              onChange={event => setName(event.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="new-agent-description">
              {t('agentPage.new.what')}
            </Label>
            <Input
              id="new-agent-description"
              value={description}
              maxLength={2000}
              placeholder={t('agentPage.new.whatPlaceholder')}
              onChange={event => setDescription(event.target.value)}
            />
          </div>
          <fieldset className="space-y-2">
            <legend className="text-sm font-medium">
              {t('agentPage.new.template')}
            </legend>
            <RadioGroup
              value={template}
              onValueChange={value => setTemplate(value as AgentTemplate)}
              className="grid grid-cols-2 gap-2"
            >
              {AGENT_TEMPLATES.map(option => (
                <label
                  key={option}
                  htmlFor={`template-${option}`}
                  className="flex cursor-pointer items-start gap-2 rounded-md border p-2 text-sm"
                >
                  <RadioGroupItem
                    id={`template-${option}`}
                    value={option}
                    className="mt-0.5"
                  />
                  <span className="space-y-0.5">
                    <span className="block font-medium">
                      {t(`agentPage.new.templates.${option}.name`)}
                    </span>
                    <span className="block text-xs text-muted-foreground">
                      {t(`agentPage.new.templates.${option}.hint`)}
                    </span>
                  </span>
                </label>
              ))}
            </RadioGroup>
          </fieldset>
          <DialogFooter className="flex-wrap gap-2 sm:justify-between">
            <Button
              type="button"
              variant="link"
              className="px-0"
              onClick={describe}
            >
              {t('agentPage.new.describeInStudio')}
            </Button>
            <Button type="submit" disabled={!name.trim() || creating}>
              {creating && <Spinner />}
              {t('agentPage.new.create')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
