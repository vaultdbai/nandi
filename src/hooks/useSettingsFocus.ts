import { useEffect } from 'react'

/**
 * Scrolls to and briefly highlights the element marked
 * `data-settings-focus={focus}` once it has rendered
 */
export function useSettingsFocus(focus: string | undefined) {
  useEffect(() => {
    if (!focus) return
    let tries = 0
    let timer = 0
    let cleanup: (() => void) | undefined
    const find = () => {
      const target = document.querySelector<HTMLElement>(
        `[data-settings-focus="${CSS.escape(focus)}"]`
      )
      if (!target) {
        if (tries++ < 20) timer = window.setTimeout(find, 100)
        return
      }
      target.scrollIntoView({ block: 'center', behavior: 'smooth' })
      target.classList.add('ring-2', 'ring-primary')
      const clear = window.setTimeout(
        () => target.classList.remove('ring-2', 'ring-primary'),
        2000
      )
      cleanup = () => {
        window.clearTimeout(clear)
        target.classList.remove('ring-2', 'ring-primary')
      }
    }
    timer = window.setTimeout(find, 0)
    return () => {
      window.clearTimeout(timer)
      cleanup?.()
    }
  }, [focus])
}
