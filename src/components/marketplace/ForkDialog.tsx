import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { GitFork } from 'lucide-react'
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
import { forkAgent } from '@/lib/marketplace-actions'

/** Agent IDs: lowercase letters, digits, hyphens; 1–64; no leading hyphen */
const AGENT_ID = /^[a-z0-9][a-z0-9-]{0,63}$/

interface ForkDialogProps {
  /** The marketplace agent to fork; `null` closes the dialog */
  agent: { id: string; name: string } | null
  onOpenChange: (open: boolean) => void
}

/**
 * Forks a shared agent into the user's own agents under a new ID. The
 * shared copy is replaced by the fork; updates stop for it.
 */
export function ForkDialog({ agent, onOpenChange }: ForkDialogProps) {
  return (
    <Dialog open={agent !== null} onOpenChange={onOpenChange}>
      <DialogContent>
        {agent && (
          <ForkForm
            key={agent.id}
            agent={agent}
            onDone={() => onOpenChange(false)}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}

function ForkForm({
  agent,
  onDone,
}: {
  agent: { id: string; name: string }
  onDone: () => void
}) {
  const { t } = useTranslation()
  const [newId, setNewId] = useState(`my-${agent.id}`.slice(0, 64))
  const [newName, setNewName] = useState(
    t('marketplace.forkDefaultName', { name: agent.name })
  )
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  const idValid = AGENT_ID.test(newId) && newId !== agent.id
  const canSubmit = idValid && newName.trim().length > 0 && !saving

  const submit = async (event: React.FormEvent) => {
    event.preventDefault()
    if (!canSubmit) return
    setSaving(true)
    setError(null)
    const failure = await forkAgent(agent.id, newId, newName.trim())
    setSaving(false)
    if (failure) {
      setError(failure)
    } else {
      onDone()
    }
  }

  return (
    <form onSubmit={event => void submit(event)} className="grid gap-4">
      <DialogHeader>
        <DialogTitle>
          {t('marketplace.forkTitle', { name: agent.name })}
        </DialogTitle>
        <DialogDescription>
          {t('marketplace.forkDescription')}
        </DialogDescription>
      </DialogHeader>
      <div className="grid gap-2">
        <Label htmlFor="fork-id">{t('marketplace.forkId')}</Label>
        <Input
          id="fork-id"
          dir="ltr"
          value={newId}
          onChange={event => setNewId(event.target.value.toLowerCase())}
          aria-invalid={!idValid}
          className="font-mono"
        />
        {!idValid && (
          <p className="text-xs text-destructive">
            {t('marketplace.forkIdHint')}
          </p>
        )}
      </div>
      <div className="grid gap-2">
        <Label htmlFor="fork-name">{t('marketplace.forkName')}</Label>
        <Input
          id="fork-name"
          value={newName}
          onChange={event => setNewName(event.target.value)}
        />
      </div>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <DialogFooter>
        <Button type="submit" disabled={!canSubmit}>
          {saving ? <Spinner /> : <GitFork />}
          {t('marketplace.fork')}
        </Button>
      </DialogFooter>
    </form>
  )
}
