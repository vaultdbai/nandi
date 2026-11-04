import { CompanionPanel, type CompanionPanelProps } from './CompanionExpanded'

/**
 * The full-width companion: the expanded card's content with room for the
 * conversation and the three lists side by side
 */
export function CompanionOverlay(props: CompanionPanelProps) {
  return <CompanionPanel {...props} large />
}
