#!/usr/bin/env node
/**
 * Rebuilds the feature test catalog's Summary counts and Master index
 * (docs/testing/README.md) from the area files, so they never drift.
 * Keeps whatever is in the "Last verified" column.
 *
 * Usage: pnpm catalog:index
 */
import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const DIR = 'docs/testing'
const README = join(DIR, 'README.md')
const ENTRY = /^### ([A-Z]{2}-\d{2}) — (.+)$/
const MAX_CELL = 90

const readme = readFileSync(README, 'utf8').replace(/\r\n/g, '\n')
const summaryAt = readme.indexOf('## Summary')
const indexAt = readme.indexOf('## Master index')
if (summaryAt < 0 || indexAt < 0) {
  throw new Error(`${README} needs "## Summary" and "## Master index" sections`)
}

/** Areas in Summary-table order: `| Title | [file.md](./file.md) | n |` */
const areas = [
  ...readme
    .slice(summaryAt, indexAt)
    .matchAll(
      /^\| ([^|]+?) +\| \[([\w-]+\.md)\]\(\.\/[\w-]+\.md\) +\| +\d+ +\|$/gm
    ),
].map(([, title, file]) => ({ title, file }))

/** "Last verified" per ID, from the current index */
const verified = new Map(
  [
    ...readme
      .slice(indexAt)
      .matchAll(/^\| ([A-Z]{2}-\d{2}) \|.*\| ([^|]*?) *\|$/gm),
  ].map(([, id, last]) => [id, last])
)

const kind = status =>
  status.startsWith('Duplicate')
    ? 'Duplicate'
    : status.split(' (')[0].split(':')[0].trim()

/** One area file's entries: id, name, status, layer, relay ("Needs relay") */
function entries(file) {
  const lines = readFileSync(join(DIR, file), 'utf8')
    .replace(/\r\n/g, '\n')
    .split('\n')
  const found = []
  for (const line of lines) {
    const heading = line.match(ENTRY)
    if (heading) {
      found.push({
        id: heading[1],
        name: heading[2],
        status: '?',
        layer: '?',
        relay: '?',
      })
      continue
    }
    const entry = found.at(-1)
    if (!entry || line.startsWith('## ')) continue
    const duplicate = line.match(/^- \*\*Duplicate:\*\* tested under ([^.]+)\./)
    if (duplicate) {
      entry.status = `Duplicate of ${duplicate[1]}`
      entry.layer = '—'
    }
    const relay = line.match(/^- \*\*Needs relay:\*\* (yes|no)$/)
    if (relay) entry.relay = relay[1]
    const status = line.match(/^- \*\*Status:\*\* (.+)$/)
    if (status) entry.status = status[1].trim()
    const layer = line.match(/Layer: `?([a-z0-9-]+(?:\s*\+\s*[a-z0-9-]+)*)/)
    if (layer && entry.layer === '?') entry.layer = layer[1]
  }
  for (const entry of found) {
    if (entry.layer === '?' && kind(entry.status) === 'Removed')
      entry.layer = '—'
    if (entry.relay === '?') {
      throw new Error(
        `${file}: ${entry.id} needs "- **Needs relay:** yes" or "no"`
      )
    }
  }
  return found
}

const cell = text => {
  const escaped = text.replace(/\|/g, '\\|')
  return escaped.length > MAX_CELL
    ? `${escaped.slice(0, MAX_CELL - 3)}...`
    : escaped
}

const all = areas.map(area => ({ ...area, entries: entries(area.file) }))
const flat = all.flatMap(area => area.entries)
const testable = flat.filter(
  e => !['Duplicate', 'Removed'].includes(kind(e.status))
)

const seen = new Set()
for (const { id } of flat) {
  if (seen.has(id)) throw new Error(`Duplicate ID ${id}`)
  seen.add(id)
}

const count = (items, key) =>
  [
    ...items.reduce(
      (map, item) => map.set(key(item), (map.get(key(item)) ?? 0) + 1),
      new Map()
    ),
  ].sort((a, b) => b[1] - a[1])

const statusCounts = count(
  flat.filter(e => kind(e.status) !== 'Duplicate'),
  e => kind(e.status)
)
const layerCounts = count(testable, e => e.layer.split(/\s|\+/)[0])
const duplicates =
  flat.length - flat.filter(e => kind(e.status) !== 'Duplicate').length
const removed = flat.filter(e => kind(e.status) === 'Removed').length
const relayCounts = count(testable, e => e.relay)

const summary = [
  '## Summary',
  '',
  `${testable.length} features to test: all ${flat.length} IDs, minus ${duplicates} duplicates that point to the entry where the feature is tested, minus ${removed} removed features.`,
  '',
  '| Area | File | Features |',
  '| --- | --- | --- |',
  ...all.map(
    a =>
      `| ${a.title} | [${a.file}](./${a.file}) | ${a.entries.filter(e => testable.includes(e)).length} |`
  ),
  '',
  '| Status | Count |',
  '| --- | --- |',
  ...statusCounts.map(([k, v]) => `| ${k} | ${v} |`),
  '',
  '| Best automation layer | Count |',
  '| --- | --- |',
  ...layerCounts.map(([k, v]) => `| \`${k}\` | ${v} |`),
  '',
  '| Needs relay | Count |',
  '| --- | --- |',
  ...relayCounts.map(([k, v]) => `| ${k} | ${v} |`),
  '',
]

const index = [
  '## Master index',
  '',
  'Fill in **Last verified** as you test (`2026-10-22 pass`, `2026-10-22 FAIL: <short note>`). Details of each test are in the area file. `pnpm catalog:index` rebuilds this table and the Summary from the area files and keeps this column.',
  '',
  ...all.flatMap(area => [
    `### ${area.title}`,
    '',
    '| ID | Feature | Status | Layer | Needs relay | Last verified |',
    '| --- | --- | --- | --- | --- | --- |',
    ...area.entries.map(
      e =>
        `| ${e.id} | ${cell(e.name)} | ${cell(e.status)} | ${e.layer} | ${e.relay} | ${verified.get(e.id) ?? ''} |`
    ),
    '',
  ]),
]

writeFileSync(
  README,
  readme.slice(0, summaryAt) + summary.join('\n') + '\n' + index.join('\n')
)
console.log(
  `${README}: ${flat.length} IDs, ${testable.length} to test, ${areas.length} areas`
)
