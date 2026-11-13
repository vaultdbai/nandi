/**
 * `NandiClient`: everything the UI can ask of the Nandi core, independent of
 * transport. Implemented over Tauri IPC (desktop) and HTTP/WebSocket
 * (browser). Methods return the same `Result` shape the Tauri bindings use,
 * so call sites handle errors identically on both.
 */

import type {
  AgentDefinitionInput,
  AgentPlaybooks,
  Playbook,
  PlaybookInput,
  AgentInfo,
  AttachmentInfo,
  AttachmentKind,
  GitStatus,
  JsonValue,
  RecoveryError,
  Result,
} from '@/lib/bindings'
import type { commands } from '@/lib/bindings'
import type { NandiEvent } from './events'

/**
 * The generated commands. Ones declared with `rpc::commands!` in Rust work
 * on both transports as they are (see `invoke.ts`)
 */
type Commands = typeof commands

/**
 * The client: the generated commands, with the hand-written methods taking
 * the place of any command of the same name
 */
export type NandiClient = Omit<Commands, keyof HandWrittenClient> &
  HandWrittenClient

export interface Subscription {
  /**
   * Resolves once events are flowing. Load snapshots only after this, so no
   * event can slip between a snapshot read and the first listener.
   */
  ready: Promise<void>
  unsubscribe: () => void
}

/**
 * What each transport writes by hand: desktop-only calls, `subscribe`, and
 * commands not yet declared with `rpc::commands!`
 */
export interface HandWrittenClient {
  /** `tauri` (desktop webview) or `web` (browser) */
  readonly transport: 'tauri' | 'web'
  /** Features that need the desktop app */
  readonly supports: {
    /** Native file picker for attachments */
    attachments: boolean
  }

  // Agents

  // Chat

  // Queue: one turn per agent; other requests wait, lowest priority first

  // Attachments (picking needs the desktop app; see `supports`)
  pickAttachments(
    kind: AttachmentKind
  ): Promise<Result<AttachmentInfo[], string>>

  // Snapshots
  /** Opens it (or shows it in the file manager); desktop only */
  openRunFile(
    runId: string,
    path: string,
    reveal: boolean
  ): Promise<Result<null, string>>

  // Settings
  saveEmergencyData(
    filename: string,
    data: JsonValue
  ): Promise<Result<null, RecoveryError>>
  loadEmergencyData(filename: string): Promise<Result<JsonValue, RecoveryError>>
  cleanupOldRecoveryFiles(): Promise<Result<number, RecoveryError>>

  // Egress

  // Workflows (listing is `getWorkflows`)

  // Triggers (rules that start runs on schedules, files, webhooks, devices,
  // and Nandi's own events). A secret is only ever returned on create,
  // update to a webhook/device source, or rotate.
  /** Where webhook/device requests go, e.g. `http://127.0.0.1:17463` */
  getWebOrigin(): Promise<string | null>

  // Marketplace: agents and packs from shared git repos. Installed ones
  // are read-only until forked; nothing updates without the user asking.

  // Agent editor + git (wave 1, Track B). Saves go to the gold-copy files.
  /** Writes the agent's files and returns the saved agent */
  saveAgent(
    agentId: string,
    input: AgentDefinitionInput
  ): Promise<Result<AgentInfo, string>>
  /**
   * Replaces the agent's instructions.md (the user's rules). An installed
   * agent's are kept as an override that updates don't touch.
   */
  saveAgentInstructions(
    agentId: string,
    instructions: string
  ): Promise<Result<null, string>>
  /** Back to an installed agent's shipped instructions; whether it had the user's own */
  resetAgentInstructions(agentId: string): Promise<Result<boolean, string>>
  /** The agent's playbooks (`skills/`), and whether they're read-only */
  agentPlaybooks(agentId: string): Promise<Result<AgentPlaybooks, string>>
  /**
   * Validates and writes one playbook. `previous`: its old name when edited
   * or renamed, `null` for a new one
   */
  saveAgentPlaybook(
    agentId: string,
    input: PlaybookInput,
    previous: string | null
  ): Promise<Result<Playbook, string>>
  deleteAgentPlaybook(
    agentId: string,
    name: string
  ): Promise<Result<null, string>>
  /** One entity as stored in its file (`agent`, `workflow`, `trigger`, …) */
  loadEntity(kind: string, id: string): Promise<Result<JsonValue, string>>
  gitStatus(): Promise<Result<GitStatus, string>>
  /** Unified diff of one file against `HEAD` */
  gitDiff(path: string): Promise<Result<string, string>>
  /** Commits what's staged; returns the commit hash */
  gitCommit(message: string): Promise<Result<string, string>>

  // Pilots: host-side MCP servers for OS control (wave 1, Track A)

  // Runs across every agent, and the audit logs (wave 1, Track C)

  // Runtime connectors: MCP servers + OAuth2 (wave 1, Track D)

  // Wave 2A: teams, sharing, and delegation

  // Agent cards + capability index (Track A)

  // Relay + presence (Track B)

  // Delegation (Track C)

  // Teams: a shared git registry (Track D)

  // Shared views + trust (Track E)

  // Wave 2B redesign

  // Run detail + analysis (Track A)

  // Workflow builder (Track B)

  // Marketplace (Track C)

  // Companion (Track D). Window control needs the desktop app.
  toggleCompanionWindow(): Promise<Result<null, string>>
  /** Quits the desktop app (the companion's way out) */
  quitNandi(): Promise<void>
  /** Sizes the companion for `shape` and keeps it in its screen corner */
  placeCompanion(
    shape: 'collapsed' | 'expanded' | 'overlay'
  ): Promise<Result<null, string>>
  /** The bison on the desktop (desktop only) */
  setCompanionBubble(bubble: boolean): Promise<Result<null, string>>
  setCompanionPinned(pinned: boolean): Promise<Result<null, string>>
  /** Watch in companion: shows it with the Request pinned (desktop only) */
  companionFocusRequest(requestId: string): Promise<Result<null, string>>

  // Command palette (Track E)

  // Wave final

  // Local-identity freeze

  // Identity: display name + user slug in `~/.nandi/repo/NANDI.md`

  // GitHub sign-in: one token for every github.com repository

  // Optional git remote for the agents repo

  // Task lifecycle

  // Clarification

  // Gateway agent
  // Live UI (`services::live`)
  // Email (the Chief of Staff reads it to keep My notes)
  // Connector hub: grants, registry connectors, calendars and files

  // Network directory

  // Team roster, member profiles, and per-member trust

  // Groups (protocol 7, `services::groups`)

  // Wave A: capabilities

  // Wave A: the unified tool catalog

  // Wave A: capability registries (not the marketplace registries)

  // Wave A: capability presence on the relay

  // Wave B: harness adapters

  // Wave B: AWS auth, read from the user's own AWS config (never key material)
  /** `aws sso login` (or `aws login` for a console profile): the AWS CLI
   * opens the browser; poll for the outcome */

  // Wave B: Bedrock models and the LiteLLM gateway

  // Wave CD: the ask box (see docs/developer/interaction-layer.md)

  // Wave CD: Studio's artifact editor

  // Wave CD: work in flight

  // Wave EF: supervisor reassignment
  // Wave EF: manual workflow steps
  // Wave EF: `.nandi/llm.yaml`

  /** Opens `url` in the system browser */
  openExternal(url: string): Promise<void>

  /**
   * Receives every core event. Call once at boot. The web client also
   * emits `nandi://resync` after reconnecting.
   */
  subscribe(handler: (event: NandiEvent) => void): Subscription
}
