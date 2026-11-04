//! ETA estimates from `eta_history` (wave final, Track B): median and p95
//! of the last 100 completions; fewer than [`MIN_SAMPLES`] give a
//! low-confidence default.
//!
//! `eta_history.provider_agent` is the provider's card ID
//! (`<instance>/<agent>`) on both sides: the provider records its own
//! completions (and acknowledges with their median), the requester records
//! what it waited for each provider.

use rusqlite::{params, Connection};

use super::task::EtaEstimate;

/// Below this many samples an estimate is the default
pub const MIN_SAMPLES: u32 = 5;
/// Default median when there's too little history
pub const DEFAULT_MEDIAN_SECONDS: u32 = 120;
/// Default p95 when there's too little history
pub const DEFAULT_P95_SECONDS: u32 = 600;
/// Completions an estimate looks at
pub const WINDOW: u32 = 100;

/// `provider_agent`: a card ID, or a bare agent ID (any instance); `None`
/// for every provider of the capability.
pub fn estimate(
    conn: &Connection,
    capability: &str,
    provider_agent: Option<&str>,
) -> rusqlite::Result<EtaEstimate> {
    let mut durations: Vec<u32> = match provider_agent.filter(|p| !p.is_empty()) {
        Some(agent) => {
            let mut stmt = conn.prepare(
                "SELECT duration_seconds FROM eta_history
                  WHERE capability = ?1 AND (provider_agent = ?2 OR provider_agent LIKE ?3)
                  ORDER BY at DESC, rowid DESC LIMIT ?4",
            )?;
            let suffix = if agent.contains('/') {
                agent.to_string()
            } else {
                format!("%/{agent}")
            };
            let rows = stmt.query_map(params![capability, agent, suffix, WINDOW], |row| {
                row.get::<_, i64>(0)
            })?;
            rows.map(|r| r.map(clamp))
                .collect::<rusqlite::Result<_>>()?
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT duration_seconds FROM eta_history WHERE capability = ?1
                  ORDER BY at DESC, rowid DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![capability, WINDOW], |row| row.get::<_, i64>(0))?;
            rows.map(|r| r.map(clamp))
                .collect::<rusqlite::Result<_>>()?
        }
    };
    Ok(from_samples(&mut durations))
}

fn clamp(seconds: i64) -> u32 {
    seconds.clamp(0, u32::MAX as i64) as u32
}

/// Median and p95 (nearest rank); the defaults below [`MIN_SAMPLES`]
pub fn from_samples(durations: &mut [u32]) -> EtaEstimate {
    let sample_count = u32::try_from(durations.len()).unwrap_or(u32::MAX);
    if sample_count < MIN_SAMPLES {
        return EtaEstimate {
            median_seconds: DEFAULT_MEDIAN_SECONDS,
            p95_seconds: DEFAULT_P95_SECONDS,
            sample_count,
        };
    }
    durations.sort_unstable();
    let n = durations.len();
    let rank = |p: f64| ((p * n as f64).ceil() as usize).clamp(1, n) - 1;
    EtaEstimate {
        median_seconds: durations[rank(0.5)],
        p95_seconds: durations[rank(0.95)],
        sample_count,
    }
}

pub fn record(
    conn: &Connection,
    capability: &str,
    provider_agent: &str,
    duration_seconds: u32,
    at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO eta_history (capability, provider_agent, duration_seconds, at)
         VALUES (?1, ?2, ?3, ?4)",
        params![capability, provider_agent, duration_seconds, at],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    async fn seed(db: &Db, provider: &str, durations: Vec<u32>) {
        let provider = provider.to_string();
        db.call(move |conn| {
            for (i, d) in durations.into_iter().enumerate() {
                record(conn, "summarize", &provider, d, i as i64)?;
            }
            Ok(())
        })
        .await
        .unwrap();
    }

    async fn get(db: &Db, provider: Option<&str>) -> EtaEstimate {
        let provider = provider.map(str::to_string);
        db.call(move |conn| estimate(conn, "summarize", provider.as_deref()))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn no_history_gives_the_defaults() {
        let db = Db::in_memory();
        let eta = get(&db, None).await;
        assert_eq!(
            eta,
            EtaEstimate {
                median_seconds: DEFAULT_MEDIAN_SECONDS,
                p95_seconds: DEFAULT_P95_SECONDS,
                sample_count: 0
            }
        );
    }

    #[tokio::test]
    async fn four_samples_are_still_the_default_with_the_real_count() {
        let db = Db::in_memory();
        seed(&db, "i/a", vec![1, 2, 3, 4]).await;
        let eta = get(&db, Some("i/a")).await;
        assert_eq!(eta.median_seconds, DEFAULT_MEDIAN_SECONDS);
        assert_eq!(eta.sample_count, 4);
    }

    #[tokio::test]
    async fn five_samples_give_a_real_estimate() {
        let db = Db::in_memory();
        seed(&db, "i/a", vec![50, 10, 30, 20, 40]).await;
        seed(&db, "j/b", vec![1000; 5]).await;
        let eta = get(&db, Some("i/a")).await;
        assert_eq!(
            eta,
            EtaEstimate {
                median_seconds: 30,
                p95_seconds: 50,
                sample_count: 5
            }
        );
        // A bare agent ID matches its card ID
        assert_eq!(get(&db, Some("a")).await, eta);
        // Across providers
        let all = get(&db, None).await;
        assert_eq!(all.sample_count, 10);
        assert_eq!(all.p95_seconds, 1000);
    }

    #[tokio::test]
    async fn only_the_last_hundred_count() {
        let db = Db::in_memory();
        // 50 old slow ones, then 100 recent fast ones (1..=100)
        let mut durations = vec![10_000; 50];
        durations.extend(1..=100);
        seed(&db, "i/a", durations).await;
        let eta = get(&db, Some("i/a")).await;
        assert_eq!(eta.sample_count, 100);
        assert_eq!(eta.median_seconds, 50);
        assert_eq!(eta.p95_seconds, 95);
    }
}
