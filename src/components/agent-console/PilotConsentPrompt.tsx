import { useEffect, useRef, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { PilotConsentDialog } from '@/components/agent-console/PilotConsentDialog'
import { useAgents } from '@/hooks/useAgents'
import { pilotQueryKeys } from '@/hooks/usePilots'
import { client } from '@/lib/client'
import { unwrapResult } from '@/lib/tauri-bindings'

/**
 * Asks for pilot consent the first time an agent needs it: a run that
 * leaves out a declared, running pilot for lack of consent emits
 * `pilot://consent_needed`, and this opens the consent dialog. Mounted
 * once, so it works whichever view is open.
 *
 * A dismissed prompt stays dismissed for this session (an agent on a
 * trigger would otherwise ask on every run); the Pilots view and the
 * agent's Permissions tab can still grant it.
 */
export function PilotConsentPrompt() {
  const agents = useAgents()
  const pilots = useQuery({
    queryKey: pilotQueryKeys.list,
    queryFn: async () => unwrapResult(await client.listPilots()),
    staleTime: Infinity,
  })
  const [request, setRequest] = useState<{
    agentId: string
    pilotId: string
  } | null>(null)
  const dismissed = useRef(new Set<string>())
  // The dialog closes after a grant too; that close isn't a dismissal
  const granted = useRef(false)

  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (event.event !== 'pilot://consent_needed') return
      const { agentId, pilotId } = event.payload
      if (dismissed.current.has(`${agentId}/${pilotId}`)) return
      // One prompt at a time; the next run asks again for the rest
      setRequest(current => current ?? { agentId, pilotId })
    })
    return () => subscription.unsubscribe()
  }, [])

  const pilot = pilots.data?.find(p => p.id === request?.pilotId) ?? null
  const agentName =
    agents.data?.find(a => a.id === request?.agentId)?.name ?? request?.agentId
  const agent =
    request && agentName ? { id: request.agentId, name: agentName } : null

  return (
    <PilotConsentDialog
      agent={pilot ? agent : null}
      pilot={agent ? pilot : null}
      onOpenChange={open => {
        if (open || !request) return
        if (!granted.current) {
          dismissed.current.add(`${request.agentId}/${request.pilotId}`)
        }
        granted.current = false
        setRequest(null)
      }}
      onGranted={() => {
        granted.current = true
      }}
    />
  )
}
