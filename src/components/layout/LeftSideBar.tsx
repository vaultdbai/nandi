import { cn } from '@/lib/utils'

interface LeftSideBarProps {
  children?: React.ReactNode
  className?: string
}

export function LeftSideBar({ children, className }: LeftSideBarProps) {
  return (
    <div
      className={cn('flex h-full flex-col border-e bg-background', className)}
    >
      {children}
    </div>
  )
}
