import { useState } from 'react'
import { useTranslation } from 'react-i18next'
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
import { Spinner } from '@/components/ui/spinner'
import { Textarea } from '@/components/ui/textarea'
import { useSaveCapability } from '@/hooks/useCapabilities'
import { rawError } from '@/lib/friendly-error'
import type { CapabilityDefinition } from '@/lib/bindings'

/** "a, b,, c " → ["a", "b", "c"] */
function splitList(value: string): string[] {
  return value
    .split(',')
    .map(item => item.trim())
    .filter(item => item !== '')
}

/**
 * Edits what the capability is: name, description, version, tags, tools,
 * and the markdown body. Policy has its own tab; ownership its own dialog.
 */
export function EditCapabilityDialog({
  definition,
  open,
  onOpenChange,
}: {
  definition: CapabilityDefinition
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const save = useSaveCapability()
  const [name, setName] = useState(definition.name)
  const [description, setDescription] = useState(definition.description)
  const [version, setVersion] = useState(definition.version)
  const [tags, setTags] = useState((definition.tags ?? []).join(', '))
  const [requiredTools, setRequiredTools] = useState(
    (definition.requiredTools ?? []).join(', ')
  )
  const [body, setBody] = useState(definition.body ?? '')

  const submit = () =>
    save.mutate(
      {
        ...definition,
        name: name.trim(),
        description: description.trim(),
        version: version.trim(),
        tags: splitList(tags),
        requiredTools: splitList(requiredTools),
        body,
      },
      {
        onSuccess: () => {
          toast.success(t('capabilities.edit.saved'))
          onOpenChange(false)
        },
        onError: error =>
          toast.error(t('capabilities.saveFailed', { error: rawError(error) })),
      }
    )

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <form
          className="flex flex-col gap-4"
          onSubmit={event => {
            event.preventDefault()
            submit()
          }}
        >
          <DialogHeader>
            <DialogTitle>{t('capabilities.edit.title')}</DialogTitle>
            <DialogDescription>{definition.id}</DialogDescription>
          </DialogHeader>
          <Field id="edit-name" label={t('capabilities.edit.name')}>
            <Input
              id="edit-name"
              value={name}
              onChange={event => setName(event.target.value)}
            />
          </Field>
          <Field id="edit-version" label={t('capabilities.edit.version')}>
            <Input
              id="edit-version"
              value={version}
              onChange={event => setVersion(event.target.value)}
            />
          </Field>
          <Field
            id="edit-description"
            label={t('capabilities.edit.description')}
          >
            <Textarea
              id="edit-description"
              rows={2}
              value={description}
              onChange={event => setDescription(event.target.value)}
            />
          </Field>
          <Field id="edit-tags" label={t('capabilities.edit.tags')}>
            <Input
              id="edit-tags"
              value={tags}
              placeholder="python, review"
              onChange={event => setTags(event.target.value)}
            />
          </Field>
          <Field id="edit-tools" label={t('capabilities.edit.requiredTools')}>
            <Input
              id="edit-tools"
              value={requiredTools}
              placeholder="shell, read_file"
              onChange={event => setRequiredTools(event.target.value)}
            />
          </Field>
          <Field id="edit-body" label={t('capabilities.edit.body')}>
            <Textarea
              id="edit-body"
              rows={6}
              className="font-mono text-xs"
              value={body}
              onChange={event => setBody(event.target.value)}
            />
          </Field>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
            >
              {t('capabilities.cancel')}
            </Button>
            <Button type="submit" disabled={save.isPending || !name.trim()}>
              {save.isPending && <Spinner />}
              {t('capabilities.edit.save')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

function Field({
  id,
  label,
  children,
}: {
  id: string
  label: string
  children: React.ReactNode
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      {children}
    </div>
  )
}
