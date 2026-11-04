import { useTranslation } from 'react-i18next'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { useNavigationStore } from '@/store/navigation-store'

/**
 * "Discard changes?" when leaving a view with unsaved changes (the view
 * sets `leaveGuarded`). Mounted once, beside the main pane.
 */
export function LeaveGuardDialog() {
  const { t } = useTranslation()
  const pending = useNavigationStore(state => state.pendingLeave)

  return (
    <AlertDialog
      open={pending !== null}
      onOpenChange={open => {
        if (!open) useNavigationStore.getState().cancelLeave()
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t('leaveGuard.title')}</AlertDialogTitle>
          <AlertDialogDescription>
            {t('leaveGuard.description')}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t('leaveGuard.stay')}</AlertDialogCancel>
          <AlertDialogAction
            onClick={() => useNavigationStore.getState().confirmLeave()}
          >
            {t('leaveGuard.discard')}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
