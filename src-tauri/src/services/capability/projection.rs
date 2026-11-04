//! Capability files into `capabilities_projection` (wave A, Track A).
//! Rebuildable from the gold copy (and the registry clones), like the rest
//! of SQLite. The watcher routes `.nandi/capabilities/` changes to
//! `CapabilityService::reload_entity`, which calls these and emits
//! `capability://created|updated|deleted`.
//!
//! One row per ID: a local definition and a registry's never share a row
//! (the service merges them, local first), so `id` stays the primary key
//! and a registry row is only written for IDs no local file claims.

use rusqlite::{params, Connection, OptionalExtension};

use super::definition::CapabilityDefinition;
use crate::db::{now_ms, Db};

/// Bumped when the projected shape changes (forces a re-projection)
pub const PROJECTION_VERSION: i64 = 1;

/// Whether an upsert created the row or changed it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projected {
    Created,
    Updated,
    Unchanged,
}

/// `registry_origin`: `None` for local definitions. A registry definition
/// never replaces a local row (`Unchanged`).
pub async fn upsert(
    db: &Db,
    definition: &CapabilityDefinition,
    source_path: &str,
    registry_origin: Option<&str>,
) -> Result<Projected, String> {
    let definition = definition.clone();
    let source_path = source_path.to_string();
    let origin = registry_origin.map(str::to_string);
    db.call(move |conn| {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let outcome = upsert_in(&tx, &definition, &source_path, origin.as_deref())?;
        tx.commit()?;
        Ok(outcome)
    })
    .await
}

/// `true` when a row was removed
pub async fn remove(db: &Db, id: &str) -> Result<bool, String> {
    let id = id.to_string();
    db.call(move |conn| {
        conn.execute("DELETE FROM capabilities_projection WHERE id = ?1", [id])
            .map(|n| n > 0)
    })
    .await
}

/// Replaces every row with `registry_origin` (`None`: the local ones) by
/// `definitions` (`(definition, source_path)`)
pub async fn replace_origin(
    db: &Db,
    registry_origin: Option<&str>,
    definitions: &[(CapabilityDefinition, String)],
) -> Result<(), String> {
    let origin = registry_origin.map(str::to_string);
    let definitions = definitions.to_vec();
    db.call(move |conn| {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        match origin.as_deref() {
            None => tx.execute(
                "DELETE FROM capabilities_projection WHERE registry_origin IS NULL",
                [],
            )?,
            Some(origin) => tx.execute(
                "DELETE FROM capabilities_projection WHERE registry_origin = ?1",
                [origin],
            )?,
        };
        for (definition, source_path) in &definitions {
            upsert_in(&tx, definition, source_path, origin.as_deref())?;
        }
        tx.commit()
    })
    .await
}

/// Every projected definition, sorted by ID
pub async fn list(db: &Db) -> Result<Vec<ProjectedCapability>, String> {
    db.call(|conn| {
        let mut stmt = conn.prepare(
            "SELECT definition_json, source_path, registry_origin
             FROM capabilities_projection ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .filter_map(
                |(json, source_path, registry_origin)| match serde_json::from_str(&json) {
                    Ok(definition) => Some(ProjectedCapability {
                        definition,
                        source_path,
                        registry_origin,
                    }),
                    Err(e) => {
                        log::warn!("Skipping an unreadable projected capability: {e}");
                        None
                    }
                },
            )
            .collect())
    })
    .await
}

/// One projected definition
pub async fn get(db: &Db, id: &str) -> Result<Option<ProjectedCapability>, String> {
    let id = id.to_string();
    let found = db
        .call(move |conn| {
            conn.query_row(
                "SELECT definition_json, source_path, registry_origin
                 FROM capabilities_projection WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
        })
        .await?;
    found
        .map(|(json, source_path, registry_origin)| {
            serde_json::from_str(&json)
                .map(|definition| ProjectedCapability {
                    definition,
                    source_path,
                    registry_origin,
                })
                .map_err(|e| format!("Unreadable projected capability: {e}"))
        })
        .transpose()
}

fn upsert_in(
    conn: &Connection,
    definition: &CapabilityDefinition,
    source_path: &str,
    origin: Option<&str>,
) -> rusqlite::Result<Projected> {
    let json = serde_json::to_string(definition)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let existing: Option<(String, Option<String>, Option<i64>)> = conn
        .query_row(
            "SELECT definition_json, registry_origin, projection_version
             FROM capabilities_projection WHERE id = ?1",
            [&definition.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let outcome = match &existing {
        None => Projected::Created,
        // Local wins: a registry never overwrites a local row
        Some((_, None, _)) if origin.is_some() => return Ok(Projected::Unchanged),
        Some((old, old_origin, version))
            if *old == json
                && old_origin.as_deref() == origin
                && *version == Some(PROJECTION_VERSION) =>
        {
            return Ok(Projected::Unchanged)
        }
        Some(_) => Projected::Updated,
    };
    conn.execute(
        "INSERT INTO capabilities_projection (id, version, name, description, owner_kind,
            owner_id, owner_display_name, definition_json, source_path, registry_origin,
            projection_version, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(id) DO UPDATE SET
            version = excluded.version, name = excluded.name,
            description = excluded.description, owner_kind = excluded.owner_kind,
            owner_id = excluded.owner_id, owner_display_name = excluded.owner_display_name,
            definition_json = excluded.definition_json, source_path = excluded.source_path,
            registry_origin = excluded.registry_origin,
            projection_version = excluded.projection_version, updated_at = excluded.updated_at",
        params![
            definition.id,
            definition.version,
            definition.name,
            definition.description,
            serde_json::to_value(definition.owner.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string)),
            definition.owner.id,
            definition.owner.display_name,
            json,
            source_path,
            origin,
            PROJECTION_VERSION,
            now_ms(),
        ],
    )?;
    Ok(outcome)
}

/// A `capabilities_projection` row
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectedCapability {
    pub definition: CapabilityDefinition,
    pub source_path: Option<String>,
    pub registry_origin: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::capability::definition;

    fn sample(id: &str, name: &str) -> CapabilityDefinition {
        definition::parse(&format!(
            "id: {id}\nname: {name}\ndescription: d\nversion: 1.0.0\nowner: {{ kind: user, id: me, display_name: Me }}\n"
        ))
        .unwrap()
    }

    #[tokio::test]
    async fn upserts_report_created_updated_unchanged() {
        let db = Db::in_memory();
        let a = sample("a", "A");
        assert_eq!(
            upsert(&db, &a, "a.yaml", None).await.unwrap(),
            Projected::Created
        );
        assert_eq!(
            upsert(&db, &a, "a.yaml", None).await.unwrap(),
            Projected::Unchanged
        );
        let renamed = sample("a", "A2");
        assert_eq!(
            upsert(&db, &renamed, "a.yaml", None).await.unwrap(),
            Projected::Updated
        );
        let rows = list(&db).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].definition.name, "A2");
        assert!(remove(&db, "a").await.unwrap());
        assert!(!remove(&db, "a").await.unwrap());
        assert!(list(&db).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn local_rows_win_and_origins_replace_independently() {
        let db = Db::in_memory();
        upsert(&db, &sample("shared", "Local"), "shared.yaml", None)
            .await
            .unwrap();
        let reg = "https://example.com/reg.git";
        replace_origin(
            &db,
            Some(reg),
            &[
                (
                    sample("shared", "Registry"),
                    "capabilities/shared.yaml".into(),
                ),
                (sample("only-reg", "R"), "capabilities/only-reg.yaml".into()),
            ],
        )
        .await
        .unwrap();
        let shared = get(&db, "shared").await.unwrap().unwrap();
        assert_eq!(shared.definition.name, "Local");
        assert_eq!(shared.registry_origin, None);
        let only = get(&db, "only-reg").await.unwrap().unwrap();
        assert_eq!(only.registry_origin.as_deref(), Some(reg));

        // Replacing the local rows leaves the registry's alone
        replace_origin(&db, None, &[]).await.unwrap();
        let ids: Vec<String> = list(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.definition.id)
            .collect();
        assert_eq!(ids, ["only-reg"]);
        replace_origin(&db, Some(reg), &[]).await.unwrap();
        assert!(list(&db).await.unwrap().is_empty());
    }
}
