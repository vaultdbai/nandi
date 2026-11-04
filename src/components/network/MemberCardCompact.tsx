import { useTranslation } from 'react-i18next'
import { useState } from 'react'
import { Bot, Copy, MessageSquare, Sparkles } from 'lucide-react'
import { toast } from 'sonner'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import type { NetworkMember } from '@/lib/bindings'
import { formatRelativeTime } from '@/lib/format-time'
import { askAvailability } from './ask-model'
import { AskMemberComposer } from './AskMemberComposer'
import { MemberAvailability } from './MemberAvailability'
import { MemberAvatar } from './MemberAvatar'

export interface MemberCardCompactProps {
  member: NetworkMember
  /** Opens the member profile */
  onOpen: (instanceId: string) => void
}

/** One network member, compact (directory grid): who, presence, what their gateway offers, and Ask */
export function MemberCardCompact({ member, onOpen }: MemberCardCompactProps) {
  const { t, i18n } = useTranslation()
  const [asking, setAsking] = useState(false)
  const { gateway } = member
  const availability = askAvailability(member)
  const lastSeen = member.online
    ? t('network.online')
    : member.lastSeen === null
      ? t('network.neverSeen')
      : t('network.lastSeen', {
          time: formatRelativeTime(member.lastSeen, i18n.language),
        })

  return (
    <Card data-testid={`member-${member.instanceId}`} className="gap-3">
      <CardHeader className="flex flex-row items-center gap-3">
        <MemberAvatar
          name={member.displayName}
          avatarUrl={member.avatarUrl}
          online={member.online}
        />
        <div className="min-w-0 flex-1">
          <CardTitle className="truncate">
            {member.displayName || t('network.unnamed')}
          </CardTitle>
          <CardDescription className="truncate" dir="auto">
            {member.handle ? `@${member.handle}` : t('network.unnamed')}
          </CardDescription>
        </div>
        {gateway.directDelegation === false && (
          <Badge variant="outline" className="shrink-0">
            {t('network.people.gatewayOnly')}
          </Badge>
        )}
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        {gateway.description && (
          <p className="line-clamp-2 text-sm text-muted-foreground">
            {gateway.description}
          </p>
        )}
        <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
          <span className="inline-flex items-center gap-1">
            <Sparkles className="size-3.5" aria-hidden />
            {t('network.capabilityCount', {
              count: gateway.capabilities.length,
            })}
          </span>
          <span className="inline-flex items-center gap-1">
            <Bot className="size-3.5" aria-hidden />
            {t('network.agentCount', { count: gateway.agentCount })}
          </span>
          <span>{lastSeen}</span>
        </div>
        <MemberAvailability availability={gateway.availability} />
      </CardContent>
      <CardFooter className="mt-auto gap-2">
        <Button
          size="sm"
          variant="outline"
          onClick={() => onOpen(member.instanceId)}
        >
          {t('network.openProfile')}
        </Button>
        <Popover open={asking} onOpenChange={setAsking}>
          <PopoverTrigger asChild>
            <Button
              size="sm"
              variant="ghost"
              disabled={availability !== 'available'}
              title={
                availability === 'available'
                  ? t('members.ask.title', { name: member.displayName })
                  : t(`members.ask.unavailable.${availability}`, {
                      name: member.displayName,
                    })
              }
            >
              <MessageSquare />
              {t('network.ask')}
            </Button>
          </PopoverTrigger>
          <PopoverContent align="start" className="w-80">
            <AskMemberComposer
              compact
              instanceId={member.instanceId}
              name={member.displayName}
              online={member.online}
              gateway={gateway}
              onSent={() => setAsking(false)}
            />
          </PopoverContent>
        </Popover>
        {!member.handle && (
          <Button
            size="sm"
            variant="ghost"
            className="ms-auto"
            onClick={() => {
              void navigator.clipboard?.writeText(member.instanceId)
              toast.success(t('network.copied'))
            }}
          >
            <Copy />
            {t('network.people.copyId')}
          </Button>
        )}
      </CardFooter>
    </Card>
  )
}
