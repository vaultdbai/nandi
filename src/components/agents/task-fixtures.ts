import type { NetworkSnapshot, TaskRecord } from '@/lib/bindings'

/** A task for tests: `writer` asked `peer/papers` to `find-papers` */
export function taskFixture(overrides: Partial<TaskRecord> = {}): TaskRecord {
  const now = Date.now()
  return {
    id: 'task-1',
    requesterInstance: 'me',
    requesterAgent: 'writer',
    providerInstance: 'peer',
    providerAgent: 'papers',
    capability: 'find-papers',
    input: { query: 'graphs' },
    state: 'working',
    etaSeconds: 60,
    deadline: null,
    priority: 0,
    createdAt: now - 10_000,
    updatedAt: now - 5_000,
    acknowledgedAt: now - 9_000,
    completedAt: null,
    error: null,
    result: null,
    parentTaskId: null,
    sourceKind: 'direct',
    attemptNumber: 1,
    previousAttemptId: null,
    ...overrides,
  }
}

/** A directory with the fixture task's provider (`peer`, "Peer Person") */
export function networkSnapshot(online: boolean): NetworkSnapshot {
  return {
    relayUrl: 'wss://relay.example',
    relayConnected: true,
    fetchedAt: Date.now(),
    members: [
      {
        instanceId: 'peer',
        displayName: 'Peer Person',
        avatarUrl: null,
        online,
        lastSeen: null,
        handle: null,
        gateway: {
          instanceId: 'peer',
          endpoint: '',
          displayName: 'Peer Person',
          description: '',
          capabilities: [],
          acceptsDelegation: true,
          maxConcurrent: 1,
          currentLoad: 0,
          owner: { githubHandle: 'peer', displayName: 'Peer Person' },
          federationEnabled: false,
          gatewayVersion: 1,
        },
      },
    ],
  }
}
