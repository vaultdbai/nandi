/** Cross-window event: the companion asks the main window to open a `View` */
export const NAVIGATE_EVENT = 'companion://navigate'

/** Cross-window event: the companion asks for a Settings pane (`{ pane }`) */
export const OPEN_SETTINGS_EVENT = 'companion://open_settings'

/** Backend → companion: pin a Request (`{ requestId }`, Watch in companion) */
export const FOCUS_REQUEST_EVENT = 'companion://focus_request'

/** Backend → companion: open the card (the hotkey, Open from elsewhere) */
export const EXPAND_EVENT = 'companion://expand'

/** Backend → companion: back to the bison (the hotkey with the card in front) */
export const COLLAPSE_EVENT = 'companion://collapse'
