use clickhouse::Client;

const MIGRATIONS: &[(u32, &str)] = &[(
    1,
    include_str!("../migrations/0001_trades_gaps_footprint.sql"),
)];

pub(crate) async fn apply(client: &Client) -> clickhouse::error::Result<()> {
    client
        .query(
            "CREATE TABLE IF NOT EXISTS schema_migrations \
             (version UInt32, applied_at DateTime DEFAULT now()) \
             ENGINE = MergeTree ORDER BY version",
        )
        .execute()
        .await?;
    let applied = client
        .query("SELECT version FROM schema_migrations")
        .fetch_all::<u32>()
        .await?;

    for &(version, sql) in MIGRATIONS {
        if applied.contains(&version) {
            continue;
        }
        for statement in statements(sql) {
            client.query(&statement).execute().await?;
        }
        client
            .query("INSERT INTO schema_migrations (version) VALUES (?)")
            .bind(version)
            .execute()
            .await?;
        tracing::info!(version, "applied migration");
    }
    Ok(())
}

fn statements(sql: &str) -> Vec<String> {
    let without_comments: String = sql
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    without_comments
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_migration_splits_into_create_statements() {
        for (_, sql) in MIGRATIONS {
            let parsed = statements(sql);
            assert!(!parsed.is_empty());
            assert!(
                parsed.iter().all(|s| s.starts_with("CREATE")),
                "{parsed:#?}"
            );
        }
    }
}
