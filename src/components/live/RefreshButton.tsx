import { useTranslation } from 'react-i18next'
import { RefreshCw } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { useRefreshFromDisk } from '@/hooks/useLiveChanges'
import { cn } from '@/lib/utils'

/**
 * Refresh (F5): re-reads this screen's files from disk, for when a change
 * didn't arrive by itself (a network drive, a laptop waking up)
 */
export function RefreshButton({ className }: { className?: string }) {
  const { t } = useTranslation()
  const refresh = useRefreshFromDisk()
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          size="icon"
          variant="ghost"
          className={cn('size-8', className)}
          aria-label={t('live.refresh')}
          disabled={refresh.isPending}
          onClick={() =>
            refresh.mutate(undefined, {
              onError: error =>
                toast.error(
                  error instanceof Error ? error.message : String(error)
                ),
            })
          }
        >
          <RefreshCw
            aria-hidden
            className={cn(refresh.isPending && 'animate-spin')}
          />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{t('live.refreshHint')}</TooltipContent>
    </Tooltip>
  )
}
