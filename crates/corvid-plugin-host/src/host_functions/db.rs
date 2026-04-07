//! Host function: read-only database access (SELECT only).
//!
//! Plugins with `DbRead` capability can execute SELECT queries against
//! the agent's database. All queries are validated to ensure they are
//! read-only — non-SELECT statements are rejected before execution.

use std::sync::Arc;

use wasmtime::Linker;

use crate::loader::PluginState;
use crate::wasm_mem;

/// Trait for pluggable database query backends.
///
/// Implementations handle actual query execution. The host validates
/// SQL is SELECT-only before delegating to this trait.
pub trait DbQuery: Send + Sync {
    /// Execute a read-only SQL query and return the result as JSON.
    ///
    /// The `plugin_id` is provided for audit logging. The SQL has already
    /// been validated as SELECT-only by the host.
    fn query(&self, plugin_id: &str, sql: &str) -> Result<serde_json::Value, String>;
}

/// Thread-safe database backend wrapper.
pub type DbBackend = dyn DbQuery;

/// Validate that a SQL string is a SELECT-only query.
///
/// Rejects INSERT, UPDATE, DELETE, DROP, ALTER, CREATE, TRUNCATE,
/// ATTACH, DETACH, PRAGMA (write), VACUUM, REINDEX, and any statement
/// that modifies data or schema. Also rejects multiple statements (semicolons).
pub fn is_select_only(sql: &str) -> bool {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Reject multiple statements
    // Simple heuristic: split on `;`, ignore trailing empty
    let statements: Vec<&str> = trimmed
        .split(';')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if statements.len() != 1 {
        return false;
    }

    let upper = statements[0].to_uppercase();
    let first_word = upper.split_whitespace().next().unwrap_or("");

    matches!(first_word, "SELECT" | "WITH" | "EXPLAIN")
}

/// Link database host functions into the WASM linker.
///
/// Provides `host_db_query` in the "env" namespace.
pub fn link(linker: &mut Linker<PluginState>) -> anyhow::Result<()> {
    // host_db_query(sql_ptr, sql_len) -> ptr to length-prefixed JSON response (0 = error)
    linker.func_wrap(
        "env",
        "host_db_query",
        |mut caller: wasmtime::Caller<'_, PluginState>, sql_ptr: i32, sql_len: i32| -> i32 {
            // Read SQL string from WASM memory
            let sql = match wasm_mem::read_str(&mut caller, sql_ptr, sql_len) {
                Some(s) => s,
                None => {
                    tracing::warn!("host_db_query: failed to read SQL from WASM memory");
                    return 0;
                }
            };

            // Validate SELECT-only
            if !is_select_only(&sql) {
                tracing::warn!(
                    plugin_id = %caller.data().plugin_id,
                    sql = %sql,
                    "host_db_query: rejected non-SELECT query"
                );
                let err = serde_json::json!({"error": "only SELECT queries are allowed"});
                let bytes = serde_json::to_vec(&err).unwrap_or_default();
                return wasm_mem::write_response(&mut caller, &bytes);
            }

            let plugin_id = caller.data().plugin_id.clone();
            let db = match &caller.data().db {
                Some(d) => Arc::clone(d),
                None => {
                    tracing::error!("host_db_query: database backend not initialized");
                    return 0;
                }
            };

            // Execute query
            match db.query(&plugin_id, &sql) {
                Ok(result) => {
                    let bytes = serde_json::to_vec(&result).unwrap_or_default();
                    wasm_mem::write_response(&mut caller, &bytes)
                }
                Err(e) => {
                    tracing::warn!(
                        plugin_id = %plugin_id,
                        error = %e,
                        "host_db_query: query failed"
                    );
                    let err = serde_json::json!({"error": e});
                    let bytes = serde_json::to_vec(&err).unwrap_or_default();
                    wasm_mem::write_response(&mut caller, &bytes)
                }
            }
        },
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_queries_allowed() {
        assert!(is_select_only("SELECT * FROM plugins"));
        assert!(is_select_only("  SELECT id FROM plugins WHERE enabled = 1  "));
        assert!(is_select_only("select count(*) from messages"));
        assert!(is_select_only("WITH cte AS (SELECT 1) SELECT * FROM cte"));
        assert!(is_select_only("EXPLAIN SELECT * FROM plugins"));
    }

    #[test]
    fn non_select_rejected() {
        assert!(!is_select_only("INSERT INTO plugins VALUES (1)"));
        assert!(!is_select_only("UPDATE plugins SET enabled = 0"));
        assert!(!is_select_only("DELETE FROM plugins"));
        assert!(!is_select_only("DROP TABLE plugins"));
        assert!(!is_select_only("ALTER TABLE plugins ADD COLUMN x"));
        assert!(!is_select_only("CREATE TABLE evil (id INT)"));
        assert!(!is_select_only("PRAGMA journal_mode = WAL"));
        assert!(!is_select_only("ATTACH DATABASE ':memory:' AS evil"));
    }

    #[test]
    fn multiple_statements_rejected() {
        assert!(!is_select_only("SELECT 1; DROP TABLE plugins"));
        assert!(!is_select_only("SELECT 1; SELECT 2"));
    }

    #[test]
    fn empty_rejected() {
        assert!(!is_select_only(""));
        assert!(!is_select_only("   "));
    }

    #[test]
    fn trailing_semicolon_allowed() {
        assert!(is_select_only("SELECT 1;"));
    }
}
