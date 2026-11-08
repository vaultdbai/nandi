import type { TFunction } from 'i18next'
import { askOf } from '@/components/requests/request-model'
import type { JsonValue } from '@/lib/tauri-bindings'

/**
 * Tools with their own sentence (`toolCall.label.<name>`), and the
 * argument the sentence names (`null`: none, so always the `Plain` one)
 */
const DETAIL: Record<string, string | null> = {
  read_file: 'path',
  list_dir: 'path',
  write_file: 'path',
  find_people: 'topic',
  my_requests: null,
  delegate_to_agent: 'text',
  get_pull_request: 'url',
  get_file: 'path',
  post_review: 'url',
  web_search: 'query',
  web_read_page: 'url',
  // The run plan
  plan_set: null,
  plan_update: null,
  // Studio's own tools
  nandi_list_runs: null,
  nandi_get_run: 'run_id',
  nandi_list_requests: null,
  nandi_get_request: 'id',
  nandi_list_agents: null,
  nandi_get_agent: 'id',
  nandi_list_workflows: null,
  nandi_get_workflow: 'id',
  nandi_list_triggers: null,
  nandi_list_tools: null,
  nandi_list_capabilities: null,
  nandi_list_connectors: null,
  nandi_list_models: null,
  nandi_network_snapshot: null,
  nandi_check_artifact: null,
  nandi_list_guides: null,
  nandi_get_guide: 'id',
  nandi_start_request: null,
  nandi_save_artifact: null,
}

/** Connector tools whose names alone say too little ("Search"): their label */
const CONNECTOR_LABEL: Record<string, string> = {
  web_search__search: 'web_search',
  web_search__read_page: 'web_read_page',
}

/** `aws__batch_describe` → `{ connector: 'aws', name: 'batch_describe' }` */
function splitTool(tool: string): { connector: string | null; name: string } {
  const at = tool.indexOf('__')
  return at > 0
    ? { connector: tool.slice(0, at), name: tool.slice(at + 2) }
    : { connector: null, name: tool }
}

/** `batch_describe` → "Batch describe" */
function words(name: string): string {
  const text = name.replace(/[_-]+/g, ' ').trim()
  return text.charAt(0).toUpperCase() + text.slice(1)
}

function objectArg(args: JsonValue, key: string): JsonValue | undefined {
  if (args === null || typeof args !== 'object' || Array.isArray(args)) {
    return undefined
  }
  return (args as Record<string, JsonValue>)[key]
}

/** The ask's first line, short enough for a card's one line */
function firstLine(text: string | null): string {
  const line =
    text
      ?.split('\n')
      .find(l => l.trim())
      ?.trim() ?? ''
  return line.length > 80 ? `${line.slice(0, 79).trimEnd()}…` : line
}

function stringArg(args: JsonValue, key: string): string {
  if (args === null || typeof args !== 'object' || Array.isArray(args)) {
    return ''
  }
  const value = (args as Record<string, JsonValue>)[key]
  return typeof value === 'string' ? value.trim() : ''
}

/**
 * What a tool call does, in words people read on a card: "Read
 * my-notes/responsibilities.md", "Find who is responsible: SOC 1", "AWS:
 * Batch describe". The raw tool name stays in the card's details.
 */
export function toolLabel(tool: string, args: JsonValue, t: TFunction): string {
  const { connector, name } = splitTool(tool)
  if (connector === 'aws') {
    return t('agentConsole.toolCall.label.aws', { action: words(name) })
  }
  const label = Object.hasOwn(CONNECTOR_LABEL, tool)
    ? (CONNECTOR_LABEL[tool] ?? name)
    : name
  if (!Object.hasOwn(DETAIL, label)) return words(name)
  const key = DETAIL[label] ?? null
  // A sentence without its detail when the argument is missing
  const detail =
    label === 'delegate_to_agent'
      ? firstLine(askOf(objectArg(args, 'input')).text)
      : key
        ? stringArg(args, key)
        : ''
  return t(
    `agentConsole.toolCall.label.${label}${detail ? '' : 'Plain'}`,
    key ? { [key]: detail } : {}
  )
}
