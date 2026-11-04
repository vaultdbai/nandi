//! Per-provider circuit breakers for this Nandi's own providers:
//! consecutive failures per (provider instance, provider ID, capability)
//! in `provider_circuits`. Three in a row open the circuit for five
//! minutes (`CIRCUIT_FAILURES`, `CIRCUIT_OPEN_MS`); a success or the
//! cooldown closes it. Opening emits `task://circuit_opened`.
//!
//! Other members' circuits are the relay's: it keeps them from its ledger
//! and leaves a failing provider out of its plans.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::{CIRCUIT_FAILURES, CIRCUIT_OPEN_MS};
use crate::db::Db;
use crate::model::Timestamp;
use crate::services::a2a::task::TASK_CIRCUIT_OPENED;
use crate::services::events::{self, EventBus, NandiEvent};

/// A `provider_circuits` row
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCircuit {
    pub instance_id: String,
    pub provider_id: String,
    pub capability_id: String,
    pub consecutive_failures: u32,
    /// Epoch ms; open for `CIRCUIT_OPEN_MS` from then
    pub opened_at: Option<Timestamp>,
}

impl ProviderCircuit {
    fn closed(instance_id: &str, provider_id: &str, capability_id: &str) -> Self {
        Self {
            instance_id: instance_id.to_string(),
            provider_id: provider_id.to_string(),
            capability_id: capability_id.to_string(),
            consecutive_failures: 0,
            opened_at: None,
        }
    }

    /// Open at `now` (epoch ms)
    pub fn is_open_at(&self, now: i64) -> bool {
        self.opened_at
            .is_some_and(|opened| now - (opened as i64) < CIRCUIT_OPEN_MS)
    }
}

/// `task://circuit_opened` for a provider circuit. Carries `capability`
/// too, so listeners of the instance-level event read it the same way.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCircuitOpened {
    #[serde(flatten)]
    pub circuit: ProviderCircuit,
    pub capability: String,
}

impl NandiEvent for ProviderCircuitOpened {
    const NAME: &'static str = TASK_CIRCUIT_OPENED;
}

pub(crate) fn row(
    conn: &Connection,
    instance_id: &str,
    provider_id: &str,
    capability_id: &str,
) -> rusqlite::Result<Option<ProviderCircuit>> {
    conn.query_row(
        "SELECT consecutive_failures, opened_at FROM provider_circuits
         WHERE instance_id = ?1 AND provider_id = ?2 AND capability_id = ?3",
        params![instance_id, provider_id, capability_id],
        |row| {
            Ok(ProviderCircuit {
                instance_id: instance_id.to_string(),
                provider_id: provider_id.to_string(),
                capability_id: capability_id.to_string(),
                consecutive_failures: row.get(0)?,
                opened_at: row.get::<_, Option<i64>>(1)?.map(|ms| ms as f64),
            })
        },
    )
    .optional()
}

/// Applies one outcome at `now`; returns the row after it and whether this
/// failure opened the circuit
pub(crate) fn record_conn(
    conn: &Connection,
    instance_id: &str,
    provider_id: &str,
    capability_id: &str,
    success: bool,
    now: i64,
) -> rusqlite::Result<(ProviderCircuit, bool)> {
    let mut circuit = row(conn, instance_id, provider_id, capability_id)?
        .unwrap_or_else(|| ProviderCircuit::closed(instance_id, provider_id, capability_id));
    let mut opened = false;
    if success {
        circuit.consecutive_failures = 0;
        circuit.opened_at = None;
    } else {
        circuit.consecutive_failures += 1;
        // Opens at the third failure; after the cooldown a failed trial
        // opens it again
        if circuit.consecutive_failures >= CIRCUIT_FAILURES && !circuit.is_open_at(now) {
            circuit.opened_at = Some(now as f64);
            opened = true;
        }
    }
    conn.execute(
        "INSERT INTO provider_circuits
             (instance_id, provider_id, capability_id, consecutive_failures, opened_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(instance_id, provider_id, capability_id) DO UPDATE SET
             consecutive_failures = excluded.consecutive_failures,
             opened_at = excluded.opened_at",
        params![
            instance_id,
            provider_id,
            capability_id,
            circuit.consecutive_failures,
            circuit.opened_at.map(|ms| ms as i64),
        ],
    )?;
    Ok((circuit, opened))
}

pub(crate) fn is_open_conn(
    conn: &Connection,
    instance_id: &str,
    provider_id: &str,
    capability_id: &str,
    now: i64,
) -> rusqlite::Result<bool> {
    Ok(row(conn, instance_id, provider_id, capability_id)?.is_some_and(|c| c.is_open_at(now)))
}

/// Success closes; the third consecutive failure opens. Returns the row
/// after the update.
pub async fn record(
    db: &Db,
    instance_id: &str,
    provider_id: &str,
    capability_id: &str,
    success: bool,
) -> Result<ProviderCircuit, String> {
    record_at(
        db,
        None,
        instance_id,
        provider_id,
        capability_id,
        success,
        crate::db::now_ms(),
    )
    .await
}

/// [`record`] at `now`, emitting `task://circuit_opened` on `bus` when this
/// failure opened the circuit
pub async fn record_at(
    db: &Db,
    bus: Option<&EventBus>,
    instance_id: &str,
    provider_id: &str,
    capability_id: &str,
    success: bool,
    now: i64,
) -> Result<ProviderCircuit, String> {
    let (instance, provider, capability) = (
        instance_id.to_string(),
        provider_id.to_string(),
        capability_id.to_string(),
    );
    let (circuit, opened) = db
        .call(move |conn| record_conn(conn, &instance, &provider, &capability, success, now))
        .await?;
    if opened {
        log::warn!(
            "Circuit opened for {instance_id}/{provider_id} on '{capability_id}' after {} failures",
            circuit.consecutive_failures
        );
        if let Some(bus) = bus {
            events::emit(
                bus,
                &ProviderCircuitOpened {
                    capability: circuit.capability_id.clone(),
                    circuit: circuit.clone(),
                },
            );
        }
    }
    Ok(circuit)
}

/// Open now (`now` in epoch ms)
pub async fn is_open(
    db: &Db,
    instance_id: &str,
    provider_id: &str,
    capability_id: &str,
    now: i64,
) -> Result<bool, String> {
    let (instance, provider, capability) = (
        instance_id.to_string(),
        provider_id.to_string(),
        capability_id.to_string(),
    );
    db.call(move |conn| is_open_conn(conn, &instance, &provider, &capability, now))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opened_events(recorder: &crate::services::events::tests::RecordingSink) -> usize {
        recorder
            .names()
            .into_iter()
            .filter(|name| *name == TASK_CIRCUIT_OPENED)
            .count()
    }

    #[tokio::test]
    async fn opens_after_three_failures_and_closes_after_five_minutes() {
        let (bus, recorder) = crate::services::events::tests::bus();
        let db = bus.db.clone();
        let fail = |now: i64| {
            let (db, bus) = (db.clone(), bus.clone());
            async move {
                record_at(
                    &db,
                    Some(&bus),
                    "peer",
                    "reviewer",
                    "code-review",
                    false,
                    now,
                )
                .await
                .unwrap()
            }
        };
        let t0 = 1_000_000;
        for i in 0..2 {
            assert!(!fail(t0 + i).await.is_open_at(t0 + i));
        }
        assert!(!is_open(&db, "peer", "reviewer", "code-review", t0)
            .await
            .unwrap());
        assert_eq!(fail(t0 + 2).await.consecutive_failures, 3);
        assert!(is_open(&db, "peer", "reviewer", "code-review", t0 + 3)
            .await
            .unwrap());
        assert_eq!(opened_events(&recorder), 1);
        // Another provider on the same instance is unaffected
        assert!(!is_open(&db, "peer", "other", "code-review", t0 + 3)
            .await
            .unwrap());
        // The cooldown closes it
        let after = t0 + 2 + CIRCUIT_OPEN_MS;
        assert!(!is_open(&db, "peer", "reviewer", "code-review", after)
            .await
            .unwrap());
        // A failed trial after the cooldown opens it again
        fail(after).await;
        assert!(is_open(&db, "peer", "reviewer", "code-review", after + 1)
            .await
            .unwrap());
        assert_eq!(opened_events(&recorder), 2);
    }

    #[tokio::test]
    async fn a_success_closes_it() {
        let db = Db::in_memory();
        for _ in 0..3 {
            record(&db, "peer", "reviewer", "code-review", false)
                .await
                .unwrap();
        }
        let now = crate::db::now_ms();
        assert!(is_open(&db, "peer", "reviewer", "code-review", now)
            .await
            .unwrap());
        let closed = record(&db, "peer", "reviewer", "code-review", true)
            .await
            .unwrap();
        assert_eq!(closed.consecutive_failures, 0);
        assert!(closed.opened_at.is_none());
        assert!(!is_open(&db, "peer", "reviewer", "code-review", now)
            .await
            .unwrap());
    }
}
