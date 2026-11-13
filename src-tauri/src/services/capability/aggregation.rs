//! The evidence aggregation job (wave A, Track F): hourly, rolling 30-day
//! aggregates per capability and provider into `capability_aggregates`,
//! and ambiguity clusters from clarification traffic (`task_messages`)
//! into `capability_ambiguities`. Emits `evidence://aggregated`.
//! Capabilities and tasks without evidence rows are skipped.
//!
//! - Each pass first back-fills evidence rows a workflow ran but that were
//!   recorded without its `step_breakdown` (or as an agent): the
//!   `workflow_steps` task message says which.
//! - A task observed on both sides of this instance (local delegation)
//!   counts once: the provider row wins over the requester row.
//! - Per capability there is one row per provider and one overall row
//!   (empty provider key columns). Aggregates are `CapabilityEvidence`
//!   JSON; `cached` reads them back (Track A's `evidence` for 30+ days).
//! - Outcomes: `completed` succeeds, `timedOut` times out, `failed`,
//!   `rejected`, and `abandoned` fail; `cancelled` and `reassigned` only
//!   count. Latency percentiles (nearest rank) are over rows with a duration
//!   and an outcome (not cancelled or reassigned).
//! - Clusters: each `clarification_request` question is reduced to its
//!   content words; questions join the first cluster whose first question
//!   shares at least half of the words (Jaccard), else start one. A
//!   cluster's ID is a stable hash of its capability and first question,
//!   so re-runs update it (keeping `promoted_to_schema`), and its label
//!   is its most frequent question.
//! - `promote_into_schema` turns a cluster into an optional string
//!   property of the capability's input schema (pure; the service writes
//!   the file).

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use super::definition::{CapabilityDefinition, ProviderKind};
use super::evidence::{ended_with_outcome, CapabilityEvidence};
use super::workflow_provider::{self, WORKFLOW_STEPS_KIND};
use super::EVIDENCE_AGGREGATED;
use crate::db::{self, Db};
use crate::model::Timestamp;
use crate::services::events::{self, NandiEvent};
use crate::services::AppState;

/// Every hour
pub const AGGREGATION_INTERVAL_SECS: u64 = 60 * 60;
/// The cached window
pub const AGGREGATE_WINDOW_DAYS: u32 = 30;
/// The first pass waits this long after startup
const FIRST_PASS_DELAY: Duration = Duration::from_secs(60);
/// Shared content words (Jaccard) for a question to join a cluster
const CLUSTER_SIMILARITY: f32 = 0.5;
/// Sample questions kept per cluster
const MAX_SAMPLES: usize = 5;
/// The `task_messages.kind` of a provider's questions
const CLARIFICATION_REQUEST_KIND: &str = "clarification_request";
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// A `capability_ambiguities` row: similar clarification questions
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AmbiguityCluster {
    pub id: String,
    pub capability_id: String,
    /// e.g. "Which Python version?"
    pub cluster_label: String,
    pub sample_questions: Vec<String>,
    pub occurrence_count: u32,
    pub first_seen_at: Option<Timestamp>,
    pub last_seen_at: Option<Timestamp>,
    pub promoted_to_schema: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AggregationReport {
    pub capabilities: u32,
    pub aggregates: u32,
    pub ambiguity_clusters: u32,
    pub computed_at: Timestamp,
}

impl NandiEvent for AggregationReport {
    const NAME: &'static str = EVIDENCE_AGGREGATED;
}

/// Starts the hourly job (the first pass a minute after startup) and the
/// workflow providers (`workflow_provider::init`). Must not block.
pub fn start(state: &AppState) {
    workflow_provider::init(state);
    if cfg!(test) {
        return;
    }
    let weak = state.downgrade();
    state.runtime().spawn(async move {
        tokio::time::sleep(FIRST_PASS_DELAY).await;
        let mut interval = tokio::time::interval(Duration::from_secs(AGGREGATION_INTERVAL_SECS));
        loop {
            interval.tick().await;
            let Some(state) = weak.upgrade() else {
                return;
            };
            match run_once(state.db()).await {
                Ok(report) => {
                    log::info!(
                        "Aggregated evidence: {} capabilities, {} aggregates, {} ambiguity clusters",
                        report.capabilities,
                        report.aggregates,
                        report.ambiguity_clusters
                    );
                    events::emit(state.bus(), &report);
                }
                Err(e) => log::warn!("Evidence aggregation failed: {e}"),
            }
        }
    });
}

/// One pass (the job's body; tests call it directly)
pub async fn run_once(db: &Db) -> Result<AggregationReport, String> {
    db.call(|conn| run_pass(conn, db::now_ms()))
        .await
        .map_err(|e| format!("Evidence aggregation failed: {e}"))
}

fn run_pass(conn: &mut Connection, now: i64) -> rusqlite::Result<AggregationReport> {
    backfill_workflow_evidence(conn)?;
    let aggregates = compute_aggregates(conn, now, AGGREGATE_WINDOW_DAYS)?;
    let capabilities: BTreeSet<&str> = aggregates
        .iter()
        .map(|a| a.capability_id.as_str())
        .collect();
    let capabilities = capabilities.len() as u32;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute(
        "DELETE FROM capability_aggregates WHERE window_days = ?1",
        [AGGREGATE_WINDOW_DAYS],
    )?;
    for aggregate in &aggregates {
        let overall = aggregate.instance_id.is_empty() && aggregate.provider_id.is_empty();
        tx.execute(
            "INSERT INTO capability_aggregates
                 (capability_id, provider_instance, provider_kind, provider_id,
                  window_days, aggregate_json, computed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                aggregate.capability_id,
                aggregate.instance_id,
                if overall {
                    ""
                } else {
                    aggregate.provider_kind.as_str()
                },
                aggregate.provider_id,
                aggregate.window_days,
                serde_json::to_string(aggregate).unwrap_or_default(),
                now,
            ],
        )?;
    }
    tx.commit()?;
    let clusters = extract_ambiguities(conn)?;
    let count = clusters.len() as u32;
    store_ambiguities(conn, &clusters)?;
    Ok(AggregationReport {
        capabilities,
        aggregates: aggregates.len() as u32,
        ambiguity_clusters: count,
        computed_at: db::ts(now),
    })
}

/// Marks evidence of tasks a workflow ran as workflow evidence, with its
/// step breakdown when the row has none
fn backfill_workflow_evidence(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE capability_evidence SET
             provider_kind = 'workflow',
             step_breakdown = COALESCE(step_breakdown, (
                 SELECT m.payload FROM task_messages m
                 WHERE m.task_id = capability_evidence.task_id AND m.kind = ?1
                 ORDER BY m.id DESC LIMIT 1))
         WHERE (step_breakdown IS NULL OR provider_kind IS NOT 'workflow')
           AND EXISTS (SELECT 1 FROM task_messages m
                       WHERE m.task_id = capability_evidence.task_id AND m.kind = ?1)",
        [WORKFLOW_STEPS_KIND],
    )
}

/// One evidence row, as aggregated
struct Observation {
    capability_id: String,
    capability_version: Option<String>,
    provider_instance: String,
    provider_kind: ProviderKind,
    provider_id: String,
    status: String,
    duration_ms: Option<f64>,
    tokens_in: u64,
    tokens_out: u64,
    cost_usd: Option<f64>,
    clarification_rounds: u32,
    escalation_required: bool,
    user_rating: Option<u32>,
    observed_at: i64,
}

/// Rolling aggregates over the last `window_days`: per (capability,
/// provider) and per capability overall (empty provider fields)
pub fn compute_aggregates(
    conn: &Connection,
    now: i64,
    window_days: u32,
) -> rusqlite::Result<Vec<CapabilityEvidence>> {
    let since = now - i64::from(window_days) * DAY_MS;
    let mut stmt = conn.prepare(
        "SELECT task_id, role, capability_id, capability_version, provider_instance,
                provider_kind, provider_id, status, duration_ms, tokens_in, tokens_out,
                cost_usd, clarification_rounds, escalation_required, user_rating, observed_at
         FROM capability_evidence WHERE observed_at >= ?1
         ORDER BY observed_at",
    )?;
    let rows = stmt.query_map([since], |row| {
        let task_id: String = row.get(0)?;
        let role: String = row.get(1)?;
        let observation = Observation {
            capability_id: row.get(2)?,
            capability_version: row.get(3)?,
            provider_instance: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            provider_kind: row
                .get::<_, Option<String>>(5)?
                .as_deref()
                .and_then(ProviderKind::parse)
                .unwrap_or_default(),
            provider_id: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
            status: row.get(7)?,
            duration_ms: row.get::<_, Option<i64>>(8)?.map(|d| d.max(0) as f64),
            tokens_in: row.get::<_, Option<i64>>(9)?.unwrap_or(0).max(0) as u64,
            tokens_out: row.get::<_, Option<i64>>(10)?.unwrap_or(0).max(0) as u64,
            cost_usd: row.get(11)?,
            clarification_rounds: row.get::<_, Option<i64>>(12)?.unwrap_or(0).max(0) as u32,
            escalation_required: row.get::<_, Option<i64>>(13)?.unwrap_or(0) != 0,
            user_rating: row
                .get::<_, Option<i64>>(14)?
                .map(|r| r.clamp(0, i64::from(u32::MAX)) as u32),
            observed_at: row.get(15)?,
        };
        Ok((task_id, role, observation))
    })?;
    // One observation per task: the provider side's when both are here
    let mut by_task: BTreeMap<(String, String), (bool, Observation)> = BTreeMap::new();
    for row in rows {
        let (task_id, role, observation) = row?;
        let provider = role == "provider";
        let key = (observation.capability_id.clone(), task_id);
        match by_task.get(&key) {
            Some((true, _)) if !provider => {}
            _ => {
                by_task.insert(key, (provider, observation));
            }
        }
    }
    // (capability, instance, kind, provider); the overall row has '' parts
    type Key = (String, String, &'static str, String);
    let mut groups: BTreeMap<Key, Vec<&Observation>> = BTreeMap::new();
    for (_, observation) in by_task.values() {
        groups
            .entry((
                observation.capability_id.clone(),
                observation.provider_instance.clone(),
                observation.provider_kind.as_str(),
                observation.provider_id.clone(),
            ))
            .or_default()
            .push(observation);
        groups
            .entry((
                observation.capability_id.clone(),
                String::new(),
                "",
                String::new(),
            ))
            .or_default()
            .push(observation);
    }
    Ok(groups
        .into_iter()
        .map(|((capability_id, instance_id, kind, provider_id), rows)| {
            summarize(
                capability_id,
                instance_id,
                ProviderKind::parse(kind).unwrap_or_default(),
                provider_id,
                window_days,
                &rows,
            )
        })
        .collect())
}

fn summarize(
    capability_id: String,
    instance_id: String,
    provider_kind: ProviderKind,
    provider_id: String,
    window_days: u32,
    rows: &[&Observation],
) -> CapabilityEvidence {
    let count = rows.len() as u32;
    let status_count = |statuses: &[&str]| {
        rows.iter()
            .filter(|r| statuses.contains(&r.status.as_str()))
            .count() as u32
    };
    let mut durations: Vec<f64> = rows
        .iter()
        .filter(|r| ended_with_outcome(&r.status))
        .filter_map(|r| r.duration_ms)
        .collect();
    durations.sort_by(f64::total_cmp);
    let costs: Vec<f64> = rows.iter().filter_map(|r| r.cost_usd).collect();
    let ratings: Vec<f32> = rows
        .iter()
        .filter_map(|r| r.user_rating.map(|v| v as f32))
        .collect();
    let mean = |values: &[f32]| {
        (!values.is_empty()).then(|| values.iter().sum::<f32>() / values.len() as f32)
    };
    let rounds: Vec<f32> = rows.iter().map(|r| r.clarification_rounds as f32).collect();
    let escalations: Vec<f32> = rows
        .iter()
        .map(|r| if r.escalation_required { 1.0 } else { 0.0 })
        .collect();
    let latest = rows.iter().max_by_key(|r| r.observed_at);
    CapabilityEvidence {
        capability_id,
        capability_version: latest.and_then(|r| r.capability_version.clone()),
        instance_id,
        provider_kind,
        provider_id,
        window_days,
        count,
        success_count: status_count(&["completed"]),
        failed_count: status_count(&["failed", "rejected", "abandoned"]),
        timeout_count: status_count(&["timedOut"]),
        p50_ms: percentile(&durations, 0.50),
        p95_ms: percentile(&durations, 0.95),
        tokens_in_total: rows.iter().map(|r| r.tokens_in).sum::<u64>() as f64,
        tokens_out_total: rows.iter().map(|r| r.tokens_out).sum::<u64>() as f64,
        cost_usd_total: (!costs.is_empty()).then(|| costs.iter().sum()),
        clarification_rounds_avg: mean(&rounds),
        escalation_rate: mean(&escalations),
        user_rating_avg: mean(&ratings),
        last_observed_at: latest.map(|r| db::ts(r.observed_at)),
    }
}

/// Nearest-rank percentile of sorted values
fn percentile(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.clamp(1, sorted.len()) - 1])
}

/// The cached aggregates of `capability_id` for `window_days` (overall row
/// first), empty before the first pass
#[cfg(test)]
pub fn cached(
    conn: &Connection,
    capability_id: &str,
    window_days: u32,
) -> rusqlite::Result<Vec<CapabilityEvidence>> {
    let mut stmt = conn.prepare(
        "SELECT aggregate_json FROM capability_aggregates
         WHERE capability_id = ?1 AND window_days = ?2
         ORDER BY provider_instance, provider_kind, provider_id",
    )?;
    let rows = stmt.query_map(params![capability_id, window_days], |row| {
        row.get::<_, String>(0)
    })?;
    let mut out = Vec::new();
    for json in rows {
        if let Ok(evidence) = serde_json::from_str(&json?) {
            out.push(evidence);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Ambiguity clusters
// ---------------------------------------------------------------------------

/// Words that don't tell questions apart
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "any", "are", "be", "can", "could", "do", "does", "for", "how", "i", "if",
    "in", "is", "it", "me", "my", "of", "on", "or", "please", "should", "that", "the", "there",
    "this", "to", "use", "want", "we", "what", "which", "would", "you", "your", "like", "need",
    "with", "will", "shall", "may", "us", "our",
];

/// A question's content words, lowercased
fn content_words(question: &str) -> BTreeSet<String> {
    question
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .filter(|w| !STOPWORDS.contains(&w.as_str()))
        .collect()
}

fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let shared = a.intersection(b).count() as f32;
    shared / a.union(b).count() as f32
}

/// FNV-1a: stable across builds (unlike `DefaultHasher`)
fn stable_hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// A cluster's ID, from its capability and first question
pub fn cluster_id(capability_id: &str, first_question: &str) -> String {
    let words: Vec<String> = content_words(first_question).into_iter().collect();
    format!(
        "amb-{:016x}",
        stable_hash(&format!("{capability_id}\n{}", words.join(" ")))
    )
}

/// A question asked by a provider about a task
#[derive(Debug, Clone, PartialEq)]
pub struct AskedQuestion {
    pub capability_id: String,
    pub question: String,
    pub at: i64,
}

struct Building {
    words: BTreeSet<String>,
    first: String,
    counts: BTreeMap<String, u32>,
    order: Vec<String>,
    first_seen: i64,
    last_seen: i64,
}

/// Groups similar questions per capability (oldest first, so a cluster's
/// first question, and so its ID, stays put)
pub fn cluster_questions(mut asked: Vec<AskedQuestion>) -> Vec<AmbiguityCluster> {
    asked.sort_by_key(|q| q.at);
    let mut by_capability: BTreeMap<String, Vec<Building>> = BTreeMap::new();
    for q in asked {
        let text = q.question.trim().to_string();
        if text.is_empty() {
            continue;
        }
        let words = content_words(&text);
        let clusters = by_capability.entry(q.capability_id.clone()).or_default();
        let found = clusters
            .iter_mut()
            .find(|c| jaccard(&c.words, &words) >= CLUSTER_SIMILARITY);
        let cluster = match found {
            Some(cluster) => cluster,
            None => {
                clusters.push(Building {
                    words: words.clone(),
                    first: text.clone(),
                    counts: BTreeMap::new(),
                    order: Vec::new(),
                    first_seen: q.at,
                    last_seen: q.at,
                });
                clusters.last_mut().expect("just pushed")
            }
        };
        if !cluster.counts.contains_key(&text) {
            cluster.order.push(text.clone());
        }
        *cluster.counts.entry(text).or_default() += 1;
        cluster.first_seen = cluster.first_seen.min(q.at);
        cluster.last_seen = cluster.last_seen.max(q.at);
    }
    let mut out = Vec::new();
    for (capability_id, clusters) in by_capability {
        for cluster in clusters {
            let label = most_frequent(&cluster.order, &cluster.counts)
                .unwrap_or_else(|| cluster.first.clone());
            out.push(AmbiguityCluster {
                id: cluster_id(&capability_id, &cluster.first),
                capability_id: capability_id.clone(),
                cluster_label: label,
                sample_questions: cluster.order.iter().take(MAX_SAMPLES).cloned().collect(),
                occurrence_count: cluster.counts.values().sum(),
                first_seen_at: Some(db::ts(cluster.first_seen)),
                last_seen_at: Some(db::ts(cluster.last_seen)),
                promoted_to_schema: false,
            });
        }
    }
    out
}

/// The most frequent of `order` (first asked on ties)
fn most_frequent(order: &[String], counts: &BTreeMap<String, u32>) -> Option<String> {
    let mut best: Option<(&String, u32)> = None;
    for text in order {
        let count = counts.get(text).copied().unwrap_or(0);
        if best.is_none_or(|(_, top)| count > top) {
            best = Some((text, count));
        }
    }
    best.map(|(text, _)| text.clone())
}

/// Every question in `clarification_request` messages of tasks with a row
fn asked_questions(conn: &Connection) -> rusqlite::Result<Vec<AskedQuestion>> {
    let mut stmt = conn.prepare(
        "SELECT t.capability, m.payload, m.at FROM task_messages m
         JOIN tasks t ON t.id = m.task_id
         WHERE m.kind = ?1",
    )?;
    let rows = stmt.query_map([CLARIFICATION_REQUEST_KIND], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let mut asked = Vec::new();
    for row in rows {
        let (capability_id, payload, at) = row?;
        let payload: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
        let questions = payload
            .get("questions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for question in questions {
            if let Some(text) = question.get("question").and_then(Value::as_str) {
                asked.push(AskedQuestion {
                    capability_id: capability_id.clone(),
                    question: text.to_string(),
                    at,
                });
            }
        }
    }
    Ok(asked)
}

/// Clusters from every recorded clarification request
pub fn extract_ambiguities(conn: &Connection) -> rusqlite::Result<Vec<AmbiguityCluster>> {
    Ok(cluster_questions(asked_questions(conn)?))
}

/// Upserts clusters, keeping `promoted_to_schema` and the earliest
/// `first_seen_at`
fn store_ambiguities(conn: &mut Connection, clusters: &[AmbiguityCluster]) -> rusqlite::Result<()> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    for cluster in clusters {
        tx.execute(
            "INSERT INTO capability_ambiguities
                 (id, capability_id, cluster_label, sample_questions, occurrence_count,
                  first_seen_at, last_seen_at, promoted_to_schema)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)
             ON CONFLICT (id) DO UPDATE SET
                 cluster_label = excluded.cluster_label,
                 sample_questions = excluded.sample_questions,
                 occurrence_count = excluded.occurrence_count,
                 first_seen_at = MIN(COALESCE(first_seen_at, excluded.first_seen_at),
                                     excluded.first_seen_at),
                 last_seen_at = excluded.last_seen_at",
            params![
                cluster.id,
                cluster.capability_id,
                cluster.cluster_label,
                serde_json::to_string(&cluster.sample_questions).unwrap_or_default(),
                cluster.occurrence_count,
                cluster.first_seen_at.map(|t| t as i64),
                cluster.last_seen_at.map(|t| t as i64),
            ],
        )?;
    }
    tx.commit()
}

fn read_cluster(row: &rusqlite::Row<'_>) -> rusqlite::Result<AmbiguityCluster> {
    let samples: Option<String> = row.get(3)?;
    Ok(AmbiguityCluster {
        id: row.get(0)?,
        capability_id: row.get(1)?,
        cluster_label: row.get(2)?,
        sample_questions: samples
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        occurrence_count: row.get::<_, Option<i64>>(4)?.unwrap_or(0).max(0) as u32,
        first_seen_at: row.get::<_, Option<i64>>(5)?.map(db::ts),
        last_seen_at: row.get::<_, Option<i64>>(6)?.map(db::ts),
        promoted_to_schema: row.get::<_, Option<i64>>(7)?.unwrap_or(0) != 0,
    })
}

const CLUSTER_COLUMNS: &str = "id, capability_id, cluster_label, sample_questions,
    occurrence_count, first_seen_at, last_seen_at, promoted_to_schema";

/// A capability's clusters, most frequent first
pub fn list_ambiguities(
    conn: &Connection,
    capability_id: &str,
) -> rusqlite::Result<Vec<AmbiguityCluster>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {CLUSTER_COLUMNS} FROM capability_ambiguities WHERE capability_id = ?1
         ORDER BY occurrence_count DESC, last_seen_at DESC, id"
    ))?;
    let rows = stmt.query_map([capability_id], read_cluster)?;
    rows.collect()
}

pub fn get_ambiguity(
    conn: &Connection,
    capability_id: &str,
    cluster_id: &str,
) -> rusqlite::Result<Option<AmbiguityCluster>> {
    conn.query_row(
        &format!(
            "SELECT {CLUSTER_COLUMNS} FROM capability_ambiguities
             WHERE capability_id = ?1 AND id = ?2"
        ),
        params![capability_id, cluster_id],
        read_cluster,
    )
    .optional()
}

pub fn mark_promoted(conn: &Connection, cluster_id: &str) -> rusqlite::Result<bool> {
    conn.execute(
        "UPDATE capability_ambiguities SET promoted_to_schema = 1 WHERE id = ?1",
        [cluster_id],
    )
    .map(|n| n > 0)
}

/// The input-schema property name a cluster becomes: its label's first
/// three content words in snake_case ("Which Python version?" →
/// `python_version`), `clarification` when it has none
pub fn property_name(cluster: &AmbiguityCluster) -> String {
    let words: Vec<String> = cluster
        .cluster_label
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .filter(|w| !STOPWORDS.contains(&w.as_str()))
        .take(3)
        .collect();
    if words.is_empty() {
        "clarification".to_string()
    } else {
        words.join("_")
    }
}

/// `definition` with the cluster as an optional string property of its
/// input schema (an object schema is created when there is none). `None`
/// when the property is already there.
pub fn promote_into_schema(
    definition: &CapabilityDefinition,
    cluster: &AmbiguityCluster,
) -> Option<CapabilityDefinition> {
    let name = property_name(cluster);
    let mut schema = definition
        .schemas
        .input
        .clone()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({ "type": "object" }));
    let fields = schema.as_object_mut()?;
    fields
        .entry("type")
        .or_insert_with(|| Value::String("object".to_string()));
    let properties = fields
        .entry("properties")
        .or_insert_with(|| json!({}))
        .as_object_mut()?;
    if properties.contains_key(&name) {
        return None;
    }
    let examples: Vec<&String> = cluster.sample_questions.iter().take(3).collect();
    properties.insert(
        name,
        json!({
            "type": "string",
            "description": format!(
                "{} (asked {} times during clarification)",
                cluster.cluster_label, cluster.occurrence_count
            ),
            "x-nandi-ambiguity": { "clusterId": cluster.id, "questions": examples },
        }),
    );
    let mut promoted = definition.clone();
    promoted.schemas.input = Some(schema);
    Some(promoted)
}

/// Promotes `cluster_id` into `definition` and marks it promoted: the
/// definition to save (`None` when it already had the field), or `Err`
/// for an unknown cluster. The caller saves before `mark_promoted`.
pub fn promotion(
    conn: &Connection,
    definition: &CapabilityDefinition,
    cluster_id: &str,
) -> Result<Option<CapabilityDefinition>, String> {
    let cluster = get_ambiguity(conn, &definition.id, cluster_id)
        .map_err(|e| format!("Database error: {e}"))?
        .ok_or_else(|| {
            format!(
                "No ambiguity cluster '{cluster_id}' for capability '{}'",
                definition.id
            )
        })?;
    Ok(promote_into_schema(definition, &cluster))
}

/// Cluster counts per capability (for summaries)
#[cfg(test)]
pub fn ambiguity_counts(
    conn: &Connection,
) -> rusqlite::Result<std::collections::HashMap<String, u32>> {
    let mut stmt = conn.prepare(
        "SELECT capability_id, COUNT(*) FROM capability_ambiguities
         WHERE COALESCE(promoted_to_schema, 0) = 0 GROUP BY capability_id",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get::<_, i64>(1)? as u32)))?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(
        conn: &Connection,
        task: &str,
        role: &str,
        provider: (&str, &str, &str),
        status: &str,
        duration_ms: i64,
        at: i64,
    ) {
        conn.execute(
            "INSERT INTO capability_evidence
                 (id, capability_id, capability_version, task_id, provider_instance,
                  provider_kind, provider_id, role, status, duration_ms, tokens_in,
                  tokens_out, cost_usd, clarification_rounds, escalation_required,
                  observed_at)
             VALUES (?1, 'cap', '1.0.0', ?2, ?3, ?4, ?5, ?6, ?7, ?8, 10, 5, 0.01, 1, 0, ?9)",
            params![
                format!("{task}-{role}"),
                task,
                provider.0,
                provider.1,
                provider.2,
                role,
                status,
                duration_ms,
                at
            ],
        )
        .unwrap();
    }

    #[tokio::test]
    async fn aggregation_computes_the_metrics() {
        let db = Db::in_memory();
        let now = db::now_ms();
        {
            let conn = db.conn().unwrap();
            let agent = ("peer", "agent", "reviewer");
            evidence(&conn, "t1", "requester", agent, "completed", 100, now - 5);
            evidence(&conn, "t2", "requester", agent, "completed", 200, now - 4);
            evidence(&conn, "t3", "requester", agent, "failed", 300, now - 3);
            evidence(&conn, "t4", "requester", agent, "timedOut", 400, now - 2);
            // Stopped, not finished: counted, but not in the latency
            evidence(&conn, "t6", "requester", agent, "cancelled", 9000, now - 2);
            // Seen from both sides here: counted once (the provider row)
            evidence(&conn, "t5", "requester", agent, "completed", 999, now - 1);
            evidence(&conn, "t5", "provider", agent, "completed", 500, now - 1);
            // Outside the window
            evidence(
                &conn,
                "old",
                "requester",
                agent,
                "failed",
                1,
                now - 31 * DAY_MS,
            );
            // A workflow provider, recorded as an agent without its steps
            let workflow = ("me", "agent", "pipeline");
            evidence(&conn, "w1", "provider", workflow, "completed", 50, now);
            conn.execute(
                "INSERT INTO task_messages (task_id, kind, payload, at)
                 VALUES ('w1', ?1, '{\"steps\":[]}', ?2)",
                params![WORKFLOW_STEPS_KIND, now],
            )
            .unwrap();
        }
        let report = run_once(&db).await.unwrap();
        assert_eq!(report.capabilities, 1);
        assert_eq!(report.aggregates, 3, "two providers and the overall row");

        let conn = db.conn().unwrap();
        let cached = cached(&conn, "cap", AGGREGATE_WINDOW_DAYS).unwrap();
        let overall = &cached[0];
        assert_eq!(
            (overall.instance_id.as_str(), overall.provider_id.as_str()),
            ("", "")
        );
        assert_eq!(overall.count, 7);
        let reviewer = cached.iter().find(|e| e.provider_id == "reviewer").unwrap();
        assert_eq!(reviewer.count, 6);
        assert_eq!(reviewer.success_count, 3);
        assert_eq!(reviewer.failed_count, 1);
        assert_eq!(reviewer.timeout_count, 1);
        assert_eq!(reviewer.p50_ms, Some(300.0));
        assert_eq!(reviewer.p95_ms, Some(500.0));
        assert_eq!(reviewer.tokens_in_total, 60.0);
        assert_eq!(reviewer.tokens_out_total, 30.0);
        assert!((reviewer.cost_usd_total.unwrap() - 0.06).abs() < 1e-9);
        assert_eq!(reviewer.clarification_rounds_avg, Some(1.0));
        assert_eq!(reviewer.escalation_rate, Some(0.0));
        assert_eq!(reviewer.user_rating_avg, None);
        assert_eq!(reviewer.capability_version.as_deref(), Some("1.0.0"));

        let pipeline = cached.iter().find(|e| e.provider_id == "pipeline").unwrap();
        assert_eq!(pipeline.provider_kind, ProviderKind::Workflow);
        let breakdown: Option<String> = conn
            .query_row(
                "SELECT step_breakdown FROM capability_evidence WHERE task_id = 'w1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(breakdown.as_deref(), Some("{\"steps\":[]}"));
        drop(conn);

        // A second pass replaces rather than duplicates
        let again = run_once(&db).await.unwrap();
        assert_eq!(again.aggregates, 3);
        let rows: i64 = db
            .conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM capability_aggregates", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(rows, 3);
    }

    #[tokio::test]
    async fn no_evidence_is_an_empty_pass() {
        let db = Db::in_memory();
        // A task without evidence rows
        db.conn()
            .unwrap()
            .execute(
                "INSERT INTO tasks (id, requester_instance, requester_agent, provider_instance,
                     provider_agent, capability, state, created_at, updated_at)
                 VALUES ('t', 'a', 'b', 'c', 'd', 'cap', 'completed', 1, 1)",
                [],
            )
            .unwrap();
        let report = run_once(&db).await.unwrap();
        assert_eq!(
            (
                report.capabilities,
                report.aggregates,
                report.ambiguity_clusters
            ),
            (0, 0, 0)
        );
    }

    fn ask(capability: &str, question: &str, at: i64) -> AskedQuestion {
        AskedQuestion {
            capability_id: capability.into(),
            question: question.into(),
            at,
        }
    }

    #[test]
    fn clusters_similar_questions() {
        let clusters = cluster_questions(vec![
            ask("code-review.python", "Which Python version?", 1),
            ask("code-review.python", "What Python version should I use?", 2),
            ask("code-review.python", "Which Python version?", 3),
            ask("code-review.python", "Which Python version do you use?", 4),
            ask("code-review.python", "Should I check type hints?", 5),
            ask("translate", "Which Python version?", 6),
        ]);
        assert_eq!(clusters.len(), 3, "{clusters:#?}");
        let python = &clusters[0];
        assert_eq!(python.capability_id, "code-review.python");
        assert_eq!(python.occurrence_count, 4);
        assert_eq!(python.cluster_label, "Which Python version?");
        assert_eq!(python.sample_questions.len(), 3);
        assert_eq!(python.first_seen_at, Some(1.0));
        assert_eq!(python.last_seen_at, Some(4.0));
        assert_eq!(clusters[1].cluster_label, "Should I check type hints?");
        // Same question, other capability: its own cluster and ID
        assert_eq!(clusters[2].capability_id, "translate");
        assert_ne!(clusters[2].id, python.id);
        assert_eq!(
            python.id,
            cluster_id("code-review.python", "Which Python version?")
        );
    }

    #[tokio::test]
    async fn extracts_clusters_from_clarification_requests() {
        let db = Db::in_memory();
        {
            let conn = db.conn().unwrap();
            for (task, at) in [("t1", 10), ("t2", 20)] {
                conn.execute(
                    "INSERT INTO tasks (id, requester_instance, requester_agent, provider_instance,
                         provider_agent, capability, state, created_at, updated_at)
                     VALUES (?1, 'a', 'b', 'c', 'd', 'code-review.python', 'completed', 1, 1)",
                    [task],
                )
                .unwrap();
                let payload = json!({
                    "taskId": task,
                    "round": 1,
                    "questions": [
                        { "id": "q1", "question": "Which Python version?", "topic": "env", "requiresHuman": false }
                    ]
                });
                conn.execute(
                    "INSERT INTO task_messages (task_id, kind, payload, at) VALUES (?1, ?2, ?3, ?4)",
                    params![task, CLARIFICATION_REQUEST_KIND, payload.to_string(), at],
                )
                .unwrap();
            }
            // Other message kinds are ignored
            conn.execute(
                "INSERT INTO task_messages (task_id, kind, payload, at)
                 VALUES ('t1', 'progress', '{\"note\": \"Which Python version?\"}', 5)",
                [],
            )
            .unwrap();
        }
        let report = run_once(&db).await.unwrap();
        assert_eq!(report.ambiguity_clusters, 1);
        let conn = db.conn().unwrap();
        let clusters = list_ambiguities(&conn, "code-review.python").unwrap();
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].occurrence_count, 2);
        assert!(!clusters[0].promoted_to_schema);

        // Promoted stays promoted across passes
        assert!(mark_promoted(&conn, &clusters[0].id).unwrap());
        drop(conn);
        run_once(&db).await.unwrap();
        let conn = db.conn().unwrap();
        let clusters = list_ambiguities(&conn, "code-review.python").unwrap();
        assert!(clusters[0].promoted_to_schema);
        assert_eq!(clusters[0].first_seen_at, Some(10.0));
        assert_eq!(
            ambiguity_counts(&conn).unwrap().get("code-review.python"),
            None
        );
    }

    #[test]
    fn promote_writes_a_new_schema_field() {
        let cluster = AmbiguityCluster {
            id: "amb-1".into(),
            capability_id: "code-review.python".into(),
            cluster_label: "Which Python version?".into(),
            sample_questions: vec!["Which Python version?".into()],
            occurrence_count: 18,
            first_seen_at: None,
            last_seen_at: None,
            promoted_to_schema: false,
        };
        assert_eq!(property_name(&cluster), "python_version");

        let mut definition = CapabilityDefinition {
            id: "code-review.python".into(),
            ..CapabilityDefinition::default()
        };
        let promoted = promote_into_schema(&definition, &cluster).unwrap();
        let input = promoted.schemas.input.clone().unwrap();
        assert_eq!(input["type"], "object");
        assert_eq!(input["properties"]["python_version"]["type"], "string");
        assert!(input["properties"]["python_version"]["description"]
            .as_str()
            .unwrap()
            .contains("asked 18 times"));
        // Optional: existing callers keep working
        assert!(input.get("required").is_none());
        // Already there: nothing to write
        assert_eq!(promote_into_schema(&promoted, &cluster), None);

        // An existing schema keeps its fields
        definition.schemas.input = Some(json!({
            "type": "object",
            "properties": { "file": { "type": "string" } },
            "required": ["file"]
        }));
        let promoted = promote_into_schema(&definition, &cluster).unwrap();
        let input = promoted.schemas.input.unwrap();
        assert_eq!(input["required"], json!(["file"]));
        assert!(input["properties"]["file"].is_object());
        assert!(input["properties"]["python_version"].is_object());
    }

    #[tokio::test]
    async fn promotion_needs_a_known_cluster() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        let definition = CapabilityDefinition {
            id: "cap".into(),
            ..CapabilityDefinition::default()
        };
        assert!(promotion(&conn, &definition, "amb-missing")
            .unwrap_err()
            .contains("No ambiguity cluster"));
    }

    #[test]
    fn percentiles_use_the_nearest_rank() {
        assert_eq!(percentile(&[], 0.5), None);
        assert_eq!(percentile(&[7.0], 0.95), Some(7.0));
        let values: Vec<f64> = (1..=20).map(f64::from).collect();
        assert_eq!(percentile(&values, 0.50), Some(10.0));
        assert_eq!(percentile(&values, 0.95), Some(19.0));
    }
}
