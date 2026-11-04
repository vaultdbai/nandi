/**
 * What a Studio edit changes, for the Discard and Commit & push
 * confirmations: lines added and removed, and a suggested commit message
 */
import type { ArtifactRef } from '@/lib/bindings'

export interface ChangeSummary {
  added: number
  removed: number
}

/** Lines added and removed (a line diff by longest common subsequence) */
export function changeSummary(before: string, after: string): ChangeSummary {
  const a = before.split('\n')
  const b = after.split('\n')
  // Rolling rows keep this O(n·m) in time but O(m) in memory
  let previous = new Array<number>(b.length + 1).fill(0)
  for (let i = 1; i <= a.length; i++) {
    const row = new Array<number>(b.length + 1).fill(0)
    for (let j = 1; j <= b.length; j++) {
      row[j] =
        a[i - 1] === b[j - 1]
          ? (previous[j - 1] ?? 0) + 1
          : Math.max(previous[j] ?? 0, row[j - 1] ?? 0)
    }
    previous = row
  }
  const common = previous[b.length] ?? 0
  return { added: b.length - common, removed: a.length - common }
}

/** "Update the writer agent", in the form a commit message takes */
export function suggestedCommitMessage(artifact: ArtifactRef): string {
  switch (artifact.kind) {
    case 'gateway':
      return 'Update NANDI.md'
    case 'agent':
      return `Update the ${artifact.id} agent`
    case 'workflow':
      return `Update the ${artifact.id} workflow`
    case 'capability':
      return `Update the ${artifact.id} capability`
    case 'policy':
      return `Update the ${artifact.id} policy`
  }
}
