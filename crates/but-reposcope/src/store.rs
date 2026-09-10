//! Independent SQLite persistence for analysis batches.

use crate::model::{AnalysisPhase, AnalysisRunStatus, AnalysisStatus, CommitFileDetail};
use anyhow::{Context as _, Result};
use parking_lot::Mutex;
use regex::Regex;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// The schema version owned by RepoScope, independent of GitButler's database.
pub const SCHEMA_VERSION: i32 = 4;

// `AnalysisStore::open` is also used by every read-only API endpoint.  Orphan
// recovery is a process-start action, not a per-query action; otherwise a
// report read during an active scan would incorrectly mark that scan as
// interrupted.  The path set is process-local, so a genuinely new desktop
// process still recovers rows left by a prior process exactly once.
static RECOVERED_DATABASES: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

/// A persisted run summary.
#[derive(Debug, Clone, Serialize)]
pub struct StoredRun {
    /// Run identifier.
    pub run_id: String,
    /// Repository path used for the pass.
    pub repository: String,
    /// Lifecycle status.
    pub status: AnalysisRunStatus,
    /// Last persisted analysis phase.
    pub phase: AnalysisPhase,
    /// Last persisted progress in the inclusive range `0..=1`.
    pub progress: f64,
    /// Number of items processed in the last reported phase.
    pub processed: u64,
    /// Estimated total items in the last reported phase.
    pub total: u64,
    /// Error text, if any.
    pub error: Option<String>,
    /// Start time in UTC milliseconds.
    pub started_at: i64,
    /// Finish time in UTC milliseconds.
    pub finished_at: Option<i64>,
    /// Ref fingerprint captured before the scan.
    pub start_refs: Option<String>,
    /// Ref fingerprint captured after the scan.
    pub end_refs: Option<String>,
    /// Worktree fingerprint captured before the scan, absent for bare repos.
    pub start_worktree: Option<String>,
    /// Worktree fingerprint captured after the scan, absent for bare repos.
    pub end_worktree: Option<String>,
}

/// The durable cache used by one project.
#[derive(Clone)]
pub struct AnalysisStore {
    path: Arc<PathBuf>,
    connection: Arc<Mutex<Connection>>,
}

impl AnalysisStore {
    /// Open `<data_dir>/reposcope.sqlite` and apply forward-only migrations.
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        let data_dir = data_dir.as_ref();
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("创建 RepoScope 数据目录失败: {}", data_dir.display()))?;
        let path = data_dir.join("reposcope.sqlite");
        let connection = Connection::open(&path)
            .with_context(|| format!("打开 RepoScope 数据库失败: {}", path.display()))?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .context("启用 RepoScope SQLite WAL 失败")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "busy_timeout", 5_000_i64)?;
        migrate(&connection)?;
        // A process can disappear while a scan is running.  Keep the active
        // successful batch untouched and make the orphaned run explicit so a
        // subsequent start can enqueue a recovery pass.  Do this only once
        // per database in this process; read-only endpoints also open the
        // store and must not interrupt a scan that is still alive.
        let recovered = RECOVERED_DATABASES.get_or_init(|| Mutex::new(HashSet::new()));
        let mut recovered = recovered.lock();
        if recovered.insert(path.clone()) {
            connection.execute(
                "UPDATE analysis_runs SET status = 'interrupted', phase = 'persisting', error = '应用退出，分析未完成' WHERE status IN ('queued', 'running')",
                [],
            )?;
        }
        Ok(Self {
            path: Arc::new(path),
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    /// Return the database file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.path.as_ref()
    }

    /// Start a run while retaining the previous active run.
    pub fn begin_run(&self, repository: &Path) -> Result<String> {
        let run_id = uuid::Uuid::new_v4().to_string();
        let now = now_ms();
        let connection = self.connection.lock();
        connection.execute(
            "INSERT INTO analysis_runs (id, repository, status, phase, progress, processed, total, started_at) VALUES (?1, ?2, 'running', 'history', 0, 0, 0, ?3)",
            params![run_id, repository.to_string_lossy(), now],
        )?;
        Ok(run_id)
    }

    /// Persist a live progress update without changing the active batch.
    pub fn update_progress(&self, status: &AnalysisStatus) -> Result<()> {
        // Terminal events share the callback channel with live progress, but
        // must never turn a completed/failed row back into `running`. The
        // terminal state is written by `finish_run` or the activation
        // transaction instead.
        if status.status != AnalysisRunStatus::Running {
            return Ok(());
        }
        let Some(run_id) = status.run_id.as_deref().filter(|value| !value.is_empty()) else {
            return Ok(());
        };
        let connection = self.connection.lock();
        connection.execute(
            "UPDATE analysis_runs SET status = 'running', phase = ?2, progress = ?3, processed = ?4, total = ?5 WHERE id = ?1 AND status IN ('queued', 'running')",
            params![
                run_id,
                phase_name(status.phase),
                status.progress.clamp(0.0, 1.0),
                sqlite_u64(status.processed),
                sqlite_u64(status.total),
            ],
        )?;
        Ok(())
    }

    /// Mark a run terminally.  This never changes `analysis_meta.active_run_id`.
    pub fn finish_run(
        &self,
        run_id: &str,
        status: AnalysisRunStatus,
        error: Option<String>,
    ) -> Result<()> {
        // Error strings can contain paths, URLs, or parser output supplied by
        // repository content.  Apply the same credential redaction boundary
        // used by the transport layer before persisting them, so a failed
        // run cannot leave a token or password in the analysis cache.
        let error = error.map(|value| sanitize_error(&value));
        let connection = self.connection.lock();
        connection.execute(
            "UPDATE analysis_runs SET status = ?2, phase = 'persisting', progress = 1, finished_at = ?3, error = ?4 WHERE id = ?1",
            params![run_id, status_name(status), now_ms(), error],
        )?;
        Ok(())
    }

    /// Atomically insert normalized core rows and activate the run.
    pub fn activate_result(
        &self,
        run_id: &str,
        result: &crate::AnalysisResult,
        _started_at: SystemTime,
    ) -> Result<()> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM analysis_snapshots WHERE run_id = ?1",
            params![run_id],
        )?;
        insert_snapshot(&transaction, run_id, "overview", &result.overview)?;
        insert_snapshot(&transaction, run_id, "activity", &result.activity)?;
        insert_snapshot(&transaction, run_id, "commits", &result.commits)?;
        insert_snapshot(&transaction, run_id, "files", &result.files)?;
        insert_snapshot(&transaction, run_id, "authors", &result.authors)?;
        insert_snapshot(&transaction, run_id, "languages", &result.languages)?;
        insert_snapshot(&transaction, run_id, "refs", &result.refs)?;
        insert_snapshot(&transaction, run_id, "couplings", &result.couplings)?;
        insert_snapshot(&transaction, run_id, "busFactor", &result.bus_factor)?;
        insert_snapshot(&transaction, run_id, "worktree", &result.worktree)?;
        insert_snapshot(&transaction, run_id, "ownership", &result.ownership)?;
        insert_snapshot(
            &transaction,
            run_id,
            "directoryOwnership",
            &result.directory_ownership,
        )?;
        insert_snapshot(&transaction, run_id, "codeAge", &result.code_age)?;
        insert_snapshot(&transaction, run_id, "fileAgeStats", &result.file_ages)?;
        insert_snapshot(
            &transaction,
            run_id,
            "ownershipCoverage",
            &result.ownership_coverage,
        )?;
        insert_snapshot(&transaction, run_id, "delivery", &result.delivery)?;
        insert_snapshot(&transaction, run_id, "footprints", &result.footprints)?;
        insert_snapshot(&transaction, run_id, "regions", &result.regions)?;
        insert_snapshot(&transaction, run_id, "dependencies", &result.dependencies)?;
        for author in &result.authors {
            transaction.execute(
                "INSERT INTO authors (run_id, identity, name, email, commit_count, files_touched, additions, deletions, owned_lines, first_commit_at, last_commit_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![run_id, author.identity, author.name, author.email, sqlite_u64(author.commit_count), sqlite_u64(author.files_touched), sqlite_u64(author.additions), sqlite_u64(author.deletions), sqlite_u64(author.owned_lines), author.first_commit_at, author.last_commit_at],
            )?;
        }
        for commit in &result.commits {
            let identity = format!("{}\0{}", commit.author_name, commit.author_email);
            transaction.execute(
                "INSERT INTO commits (run_id, oid, subject, authored_at, author_identity, additions, deletions, files_changed, category) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![run_id, commit.oid, commit.subject, commit.authored_at, identity, sqlite_u64(commit.additions), sqlite_u64(commit.deletions), sqlite_u64(commit.files_changed), commit.category],
            )?;
        }
        for (commit_oid, changes) in &result.commit_files {
            for change in changes {
                transaction.execute(
                    "INSERT INTO commit_files (run_id, commit_oid, path, previous_path, additions, deletions) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![run_id, commit_oid, change.path, change.previous_path, sqlite_u64(change.additions), sqlite_u64(change.deletions)],
                )?;
            }
        }
        for file in &result.files {
            transaction.execute(
                "INSERT INTO file_stats (run_id, path, language, tracked, lines, additions, deletions, commit_count, author_count, hotspot_score, last_changed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![run_id, file.path, file.language, file.tracked, sqlite_u64(file.lines), sqlite_u64(file.additions), sqlite_u64(file.deletions), sqlite_u64(file.commit_count), sqlite_u64(file.author_count), file.hotspot_score, file.last_changed_at],
            )?;
        }
        for day in &result.activity {
            transaction.execute(
                "INSERT INTO daily_stats (run_id, date, commits, additions, deletions, active_authors) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![run_id, day.date, sqlite_u64(day.commits), sqlite_u64(day.additions), sqlite_u64(day.deletions), sqlite_u64(day.active_authors)],
            )?;
        }
        for language in &result.languages {
            transaction.execute(
                "INSERT INTO languages (run_id, language, files, lines, percentage) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![run_id, language.language, sqlite_u64(language.files), sqlite_u64(language.lines), language.percentage],
            )?;
        }
        for reference in &result.refs {
            transaction.execute(
                "INSERT INTO refs (run_id, name, target, excluded) VALUES (?1, ?2, ?3, ?4)",
                params![run_id, reference.name, reference.target, reference.excluded],
            )?;
        }
        for coupling in &result.couplings {
            transaction.execute(
                "INSERT INTO couplings (run_id, left_path, right_path, co_changes, strength) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![run_id, coupling.left, coupling.right, sqlite_u64(coupling.co_changes), coupling.strength],
            )?;
        }
        for owner in &result.ownership {
            transaction.execute(
                "INSERT INTO ownership (run_id, path, author_identity, author_name, lines, percentage, age_days) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![run_id, owner.path, owner.author_identity, owner.author_name, sqlite_u64(owner.lines), owner.percentage, sqlite_u64(owner.age_days)],
            )?;
        }
        for age in &result.file_ages {
            transaction.execute(
                "INSERT INTO file_age_stats (run_id, path, average_days, oldest_days, newest_days, lines) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![run_id, age.path, sqlite_u64(age.average_days), sqlite_u64(age.oldest_days), sqlite_u64(age.newest_days), sqlite_u64(age.lines)],
            )?;
        }
        for bucket in &result.code_age {
            transaction.execute(
                "INSERT INTO code_age_buckets (run_id, bucket, label, files, lines, percentage) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![run_id, bucket.bucket, bucket.label, sqlite_u64(bucket.files), sqlite_u64(bucket.lines), bucket.percentage],
            )?;
        }
        transaction.execute(
            "INSERT INTO ownership_coverage (run_id, files_total, files_analyzed, lines_total, lines_analyzed, percentage) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                run_id,
                sqlite_u64(result.ownership_coverage.files_total),
                sqlite_u64(result.ownership_coverage.files_analyzed),
                sqlite_u64(result.ownership_coverage.lines_total),
                sqlite_u64(result.ownership_coverage.lines_analyzed),
                result.ownership_coverage.percentage,
            ],
        )?;
        transaction.execute(
            "INSERT OR REPLACE INTO analysis_meta (id, active_run_id, freshness, updated_at) VALUES (1, ?1, ?2, ?3)",
            params![run_id, freshness_name(result.freshness), now_ms()],
        )?;
        let terminal_status = if result.partial { "partial" } else { "complete" };
        transaction.execute(
            "UPDATE analysis_runs SET status = ?2, phase = 'persisting', progress = 1, processed = total, finished_at = ?3, error = NULL WHERE id = ?1",
            params![run_id, terminal_status, now_ms()],
        )?;
        transaction.execute(
            "UPDATE analysis_runs SET start_refs = ?2, end_refs = ?3, start_worktree = ?4, end_worktree = ?5 WHERE id = ?1",
            params![
                run_id,
                result.start_fingerprint.refs,
                result.end_fingerprint.refs,
                result.start_fingerprint.worktree,
                result.end_fingerprint.worktree,
            ],
        )?;
        transaction.execute("DELETE FROM authors WHERE run_id != ?1", params![run_id])?;
        transaction.execute("DELETE FROM commits WHERE run_id != ?1", params![run_id])?;
        transaction.execute(
            "DELETE FROM commit_files WHERE run_id != ?1",
            params![run_id],
        )?;
        transaction.execute("DELETE FROM file_stats WHERE run_id != ?1", params![run_id])?;
        transaction.execute(
            "DELETE FROM daily_stats WHERE run_id != ?1",
            params![run_id],
        )?;
        transaction.execute("DELETE FROM languages WHERE run_id != ?1", params![run_id])?;
        transaction.execute("DELETE FROM refs WHERE run_id != ?1", params![run_id])?;
        transaction.execute("DELETE FROM couplings WHERE run_id != ?1", params![run_id])?;
        transaction.execute("DELETE FROM ownership WHERE run_id != ?1", params![run_id])?;
        transaction.execute(
            "DELETE FROM ownership_coverage WHERE run_id != ?1",
            params![run_id],
        )?;
        transaction.execute(
            "DELETE FROM code_age_buckets WHERE run_id != ?1",
            params![run_id],
        )?;
        transaction.execute(
            "DELETE FROM file_age_stats WHERE run_id != ?1",
            params![run_id],
        )?;
        // Keep the last active batch available until the new transaction has
        // committed, then reclaim its serialized snapshots.  Failed or
        // cancelled runs never reach this point, so they cannot erase the
        // previous result or leave unbounded payload growth behind.
        transaction.execute(
            "DELETE FROM analysis_snapshots WHERE run_id != ?1",
            params![run_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Read the active run summary.
    pub fn active_run(&self) -> Result<Option<StoredRun>> {
        let connection = self.connection.lock();
        let run_id: Option<String> = connection
            .query_row(
                "SELECT active_run_id FROM analysis_meta WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        drop(connection);
        run_id.map_or(Ok(None), |run_id| self.run(&run_id))
    }

    /// Read a run by identifier.
    pub fn run(&self, run_id: &str) -> Result<Option<StoredRun>> {
        let connection = self.connection.lock();
        connection
            .query_row(
                "SELECT id, repository, status, phase, progress, processed, total, error, started_at, finished_at, start_refs, end_refs, start_worktree, end_worktree FROM analysis_runs WHERE id = ?1",
                params![run_id],
                |row| {
                    Ok(StoredRun {
                        run_id: row.get(0)?,
                        repository: row.get(1)?,
                        status: parse_status(row.get::<_, String>(2)?.as_str()),
                        phase: parse_phase(row.get::<_, String>(3)?.as_str()),
                        progress: row.get::<_, f64>(4)?.clamp(0.0, 1.0),
                        processed: sqlite_i64_to_u64(row.get(5)?),
                        total: sqlite_i64_to_u64(row.get(6)?),
                        error: row.get(7)?,
                        started_at: row.get(8)?,
                        finished_at: row.get(9)?,
                        start_refs: row.get(10)?,
                        end_refs: row.get(11)?,
                        start_worktree: row.get(12)?,
                        end_worktree: row.get(13)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Read the most recently queued or completed run, regardless of whether
    /// it became the active result.  This is what the UI uses to surface a
    /// failed, cancelled, or interrupted retry instead of hiding it behind an
    /// older successful batch.
    pub fn latest_run(&self) -> Result<Option<StoredRun>> {
        let connection = self.connection.lock();
        connection
            .query_row(
                "SELECT id, repository, status, phase, progress, processed, total, error, started_at, finished_at, start_refs, end_refs, start_worktree, end_worktree FROM analysis_runs ORDER BY started_at DESC LIMIT 1",
                [],
                |row| {
                    Ok(StoredRun {
                        run_id: row.get(0)?,
                        repository: row.get(1)?,
                        status: parse_status(row.get::<_, String>(2)?.as_str()),
                        phase: parse_phase(row.get::<_, String>(3)?.as_str()),
                        progress: row.get::<_, f64>(4)?.clamp(0.0, 1.0),
                        processed: sqlite_i64_to_u64(row.get(5)?),
                        total: sqlite_i64_to_u64(row.get(6)?),
                        error: row.get(7)?,
                        started_at: row.get(8)?,
                        finished_at: row.get(9)?,
                        start_refs: row.get(10)?,
                        end_refs: row.get(11)?,
                        start_worktree: row.get(12)?,
                        end_worktree: row.get(13)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Load one active snapshot using a strongly typed DTO.
    pub fn active_snapshot<T: DeserializeOwned>(&self, kind: &str) -> Result<Option<T>> {
        let connection = self.connection.lock();
        let payload: Option<String> = connection
            .query_row(
                "SELECT s.payload FROM analysis_snapshots s JOIN analysis_meta m ON m.active_run_id = s.run_id WHERE m.id = 1 AND s.kind = ?1",
                params![kind],
                |row| row.get(0),
            )
            .optional()?;
        payload
            .map(|payload| serde_json::from_str(&payload).context("解析 RepoScope 快照失败"))
            .transpose()
    }

    /// Load file-level changes for one commit from the active batch.
    pub fn active_commit_files(&self, oid: &str) -> Result<Vec<CommitFileDetail>> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT f.path, f.previous_path, f.additions, f.deletions
             FROM commit_files f
             JOIN analysis_meta m ON m.active_run_id = f.run_id
             WHERE m.id = 1 AND f.commit_oid = ?1
             ORDER BY f.path",
        )?;
        let rows = statement
            .query_map(params![oid], |row| {
                Ok(CommitFileDetail {
                    path: row.get(0)?,
                    previous_path: row.get(1)?,
                    additions: sqlite_i64_to_u64(row.get(2)?),
                    deletions: sqlite_i64_to_u64(row.get(3)?),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Return the active run id, if a successful batch exists.
    pub fn active_run_id(&self) -> Result<Option<String>> {
        let connection = self.connection.lock();
        connection
            .query_row(
                "SELECT active_run_id FROM analysis_meta WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Convert a stored run to a frontend status object.
    pub fn status(&self, run_id: Option<&str>) -> Result<Option<AnalysisStatus>> {
        let run = match run_id {
            Some(run_id) => self.run(run_id)?,
            None => self.latest_run()?,
        };
        let freshness = {
            let connection = self.connection.lock();
            connection
                .query_row(
                    "SELECT freshness FROM analysis_meta WHERE id = 1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .map(|value| parse_freshness(&value))
        };
        Ok(run.map(|run| AnalysisStatus {
            run_id: Some(run.run_id),
            status: run.status,
            phase: run.phase,
            progress: if run.finished_at.is_some() {
                1.0
            } else {
                run.progress
            },
            processed: run.processed,
            total: run.total,
            error: run.error,
            freshness,
        }))
    }
}

fn migrate(connection: &Connection) -> Result<()> {
    let mut version: i32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        anyhow::bail!("RepoScope 数据库版本过新: {version}");
    }
    if version == 0 {
        connection.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS analysis_meta (id INTEGER PRIMARY KEY CHECK (id = 1), active_run_id TEXT, freshness TEXT NOT NULL DEFAULT 'interrupted', updated_at INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS analysis_runs (id TEXT PRIMARY KEY, repository TEXT NOT NULL, status TEXT NOT NULL, phase TEXT NOT NULL, progress REAL NOT NULL DEFAULT 0, processed INTEGER NOT NULL DEFAULT 0, total INTEGER NOT NULL DEFAULT 0, error TEXT, started_at INTEGER NOT NULL, finished_at INTEGER, start_refs TEXT, end_refs TEXT, start_worktree TEXT, end_worktree TEXT);
             CREATE TABLE IF NOT EXISTS analysis_snapshots (run_id TEXT NOT NULL REFERENCES analysis_runs(id) ON DELETE CASCADE, kind TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY (run_id, kind));
             CREATE TABLE IF NOT EXISTS authors (run_id TEXT NOT NULL REFERENCES analysis_runs(id) ON DELETE CASCADE, identity TEXT NOT NULL, name TEXT NOT NULL, email TEXT NOT NULL, commit_count INTEGER NOT NULL, files_touched INTEGER NOT NULL DEFAULT 0, additions INTEGER NOT NULL, deletions INTEGER NOT NULL, owned_lines INTEGER NOT NULL, first_commit_at INTEGER, last_commit_at INTEGER, PRIMARY KEY (run_id, identity));
             CREATE TABLE IF NOT EXISTS commits (run_id TEXT NOT NULL REFERENCES analysis_runs(id) ON DELETE CASCADE, oid TEXT NOT NULL, subject TEXT NOT NULL, authored_at INTEGER NOT NULL, author_identity TEXT NOT NULL, additions INTEGER NOT NULL, deletions INTEGER NOT NULL, files_changed INTEGER NOT NULL, category TEXT NOT NULL, PRIMARY KEY (run_id, oid));
             CREATE TABLE IF NOT EXISTS commit_files (run_id TEXT NOT NULL, commit_oid TEXT NOT NULL, path TEXT NOT NULL, previous_path TEXT, additions INTEGER NOT NULL, deletions INTEGER NOT NULL, PRIMARY KEY (run_id, commit_oid, path), FOREIGN KEY (run_id, commit_oid) REFERENCES commits(run_id, oid) ON DELETE CASCADE);
             CREATE TABLE IF NOT EXISTS file_stats (run_id TEXT NOT NULL REFERENCES analysis_runs(id) ON DELETE CASCADE, path TEXT NOT NULL, language TEXT, tracked INTEGER NOT NULL DEFAULT 0, lines INTEGER NOT NULL, additions INTEGER NOT NULL, deletions INTEGER NOT NULL, commit_count INTEGER NOT NULL, author_count INTEGER NOT NULL DEFAULT 0, hotspot_score REAL NOT NULL, last_changed_at INTEGER, PRIMARY KEY (run_id, path));
             CREATE TABLE IF NOT EXISTS daily_stats (run_id TEXT NOT NULL REFERENCES analysis_runs(id) ON DELETE CASCADE, date TEXT NOT NULL, commits INTEGER NOT NULL, additions INTEGER NOT NULL, deletions INTEGER NOT NULL, active_authors INTEGER NOT NULL, PRIMARY KEY (run_id, date));
             CREATE TABLE IF NOT EXISTS languages (run_id TEXT NOT NULL REFERENCES analysis_runs(id) ON DELETE CASCADE, language TEXT NOT NULL, files INTEGER NOT NULL, lines INTEGER NOT NULL, percentage REAL NOT NULL, PRIMARY KEY (run_id, language));
             CREATE TABLE IF NOT EXISTS refs (run_id TEXT NOT NULL REFERENCES analysis_runs(id) ON DELETE CASCADE, name TEXT NOT NULL, target TEXT, excluded INTEGER NOT NULL, PRIMARY KEY (run_id, name));
             CREATE TABLE IF NOT EXISTS couplings (run_id TEXT NOT NULL REFERENCES analysis_runs(id) ON DELETE CASCADE, left_path TEXT NOT NULL, right_path TEXT NOT NULL, co_changes INTEGER NOT NULL, strength REAL NOT NULL, PRIMARY KEY (run_id, left_path, right_path));
             CREATE TABLE IF NOT EXISTS ownership (run_id TEXT NOT NULL, path TEXT NOT NULL, author_identity TEXT NOT NULL, author_name TEXT NOT NULL, lines INTEGER NOT NULL, percentage REAL NOT NULL, age_days INTEGER NOT NULL, PRIMARY KEY (run_id, path, author_identity), FOREIGN KEY (run_id) REFERENCES analysis_runs(id) ON DELETE CASCADE);
             CREATE TABLE IF NOT EXISTS ownership_coverage (run_id TEXT PRIMARY KEY REFERENCES analysis_runs(id) ON DELETE CASCADE, files_total INTEGER NOT NULL, files_analyzed INTEGER NOT NULL, lines_total INTEGER NOT NULL, lines_analyzed INTEGER NOT NULL, percentage REAL NOT NULL);
             CREATE TABLE IF NOT EXISTS code_age_buckets (run_id TEXT NOT NULL, bucket TEXT NOT NULL, label TEXT NOT NULL, files INTEGER NOT NULL, lines INTEGER NOT NULL, percentage REAL NOT NULL, PRIMARY KEY (run_id, bucket), FOREIGN KEY (run_id) REFERENCES analysis_runs(id) ON DELETE CASCADE);
             CREATE TABLE IF NOT EXISTS file_age_stats (run_id TEXT NOT NULL, path TEXT NOT NULL, average_days INTEGER NOT NULL, oldest_days INTEGER NOT NULL, newest_days INTEGER NOT NULL, lines INTEGER NOT NULL, PRIMARY KEY (run_id, path), FOREIGN KEY (run_id) REFERENCES analysis_runs(id) ON DELETE CASCADE);
             PRAGMA user_version = 4;
             COMMIT;",
        )?;
        version = 4;
    }
    if version == 1 {
        connection.execute_batch(
            "BEGIN;
             ALTER TABLE analysis_runs ADD COLUMN start_refs TEXT;
             ALTER TABLE analysis_runs ADD COLUMN end_refs TEXT;
             ALTER TABLE analysis_runs ADD COLUMN start_worktree TEXT;
             ALTER TABLE analysis_runs ADD COLUMN end_worktree TEXT;
             PRAGMA user_version = 2;
             COMMIT;",
        )?;
        version = 2;
    }
    if version == 2 {
        connection.execute_batch(
            "BEGIN;
             ALTER TABLE file_stats ADD COLUMN tracked INTEGER NOT NULL DEFAULT 0;
             PRAGMA user_version = 3;
             COMMIT;",
        )?;
        version = 3;
    }
    if version == 3 {
        connection.execute_batch(
            "BEGIN;
             ALTER TABLE authors ADD COLUMN files_touched INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE authors ADD COLUMN first_commit_at INTEGER;
             ALTER TABLE authors ADD COLUMN last_commit_at INTEGER;
             ALTER TABLE file_stats ADD COLUMN author_count INTEGER NOT NULL DEFAULT 0;
             PRAGMA user_version = 4;
             COMMIT;",
        )?;
    }
    Ok(())
}

fn insert_snapshot<T: Serialize>(
    transaction: &rusqlite::Transaction<'_>,
    run_id: &str,
    kind: &str,
    value: &T,
) -> Result<()> {
    let payload = serde_json::to_string(value).context("序列化 RepoScope 快照失败")?;
    transaction.execute(
        "INSERT OR REPLACE INTO analysis_snapshots (run_id, kind, payload) VALUES (?1, ?2, ?3)",
        params![run_id, kind, payload],
    )?;
    Ok(())
}

fn status_name(status: AnalysisRunStatus) -> &'static str {
    match status {
        AnalysisRunStatus::Queued => "queued",
        AnalysisRunStatus::Running => "running",
        AnalysisRunStatus::Complete => "complete",
        AnalysisRunStatus::Partial => "partial",
        AnalysisRunStatus::Failed => "failed",
        AnalysisRunStatus::Cancelled => "cancelled",
        AnalysisRunStatus::Interrupted => "interrupted",
    }
}

fn phase_name(phase: AnalysisPhase) -> &'static str {
    match phase {
        AnalysisPhase::History => "history",
        AnalysisPhase::Worktree => "worktree",
        AnalysisPhase::Metrics => "metrics",
        AnalysisPhase::Ownership => "ownership",
        AnalysisPhase::Evidence => "evidence",
        AnalysisPhase::Persisting => "persisting",
        AnalysisPhase::Queued => "queued",
    }
}

fn parse_status(status: &str) -> AnalysisRunStatus {
    match status {
        "queued" => AnalysisRunStatus::Queued,
        "running" => AnalysisRunStatus::Running,
        "complete" => AnalysisRunStatus::Complete,
        "partial" => AnalysisRunStatus::Partial,
        "cancelled" => AnalysisRunStatus::Cancelled,
        "interrupted" => AnalysisRunStatus::Interrupted,
        _ => AnalysisRunStatus::Failed,
    }
}

fn parse_phase(phase: &str) -> AnalysisPhase {
    match phase {
        "history" => AnalysisPhase::History,
        "worktree" => AnalysisPhase::Worktree,
        "metrics" => AnalysisPhase::Metrics,
        "ownership" => AnalysisPhase::Ownership,
        "evidence" => AnalysisPhase::Evidence,
        "queued" => AnalysisPhase::Queued,
        _ => AnalysisPhase::Persisting,
    }
}

fn freshness_name(freshness: crate::model::Freshness) -> &'static str {
    match freshness {
        crate::model::Freshness::Fresh => "fresh",
        crate::model::Freshness::HistoryStale => "historyStale",
        crate::model::Freshness::WorktreeStale => "worktreeStale",
        crate::model::Freshness::Interrupted => "interrupted",
    }
}

fn parse_freshness(value: &str) -> crate::model::Freshness {
    match value {
        "fresh" => crate::model::Freshness::Fresh,
        "historyStale" => crate::model::Freshness::HistoryStale,
        "worktreeStale" => crate::model::Freshness::WorktreeStale,
        _ => crate::model::Freshness::Interrupted,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().min(i64::MAX as u128) as i64
        })
}

// SQLite INTEGER is signed 64-bit while the public report contract uses
// unsigned counters.  Counts are bounded by filesystem/repository sizes in
// practice; saturating here keeps persistence deterministic instead of
// allowing a platform-sized counter to abort an otherwise valid batch.
fn sqlite_u64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn sqlite_i64_to_u64(value: i64) -> u64 {
    value.max(0) as u64
}

pub(crate) fn sanitize_error(value: &str) -> String {
    static URL_USERINFO: OnceLock<Regex> = OnceLock::new();
    static JDBC_USERINFO: OnceLock<Regex> = OnceLock::new();
    static BEARER: OnceLock<Regex> = OnceLock::new();
    static SECRET: OnceLock<Regex> = OnceLock::new();

    let value = URL_USERINFO
        .get_or_init(|| {
            Regex::new(r#"(?i)(\b(?:https?|ssh|git)://)[^/\s:@]+:[^/\s@]+@"#)
                .expect("valid store URL credential regex")
        })
        .replace_all(value, "$1[已脱敏]@");
    let value = JDBC_USERINFO
        .get_or_init(|| {
            Regex::new(r#"(?i)(\bjdbc:[^:\s]+(?::[^:\s]+)*:)[^/\s:@]+/[^@\s]+@"#)
                .expect("valid store JDBC credential regex")
        })
        .replace_all(&value, "$1[已脱敏]@");
    let value = BEARER
        .get_or_init(|| {
            Regex::new(r#"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+"#)
                .expect("valid store bearer credential regex")
        })
        .replace_all(&value, "Bearer [已脱敏]");
    SECRET
        .get_or_init(|| {
            Regex::new(
                r#"(?i)["']?(user(?:name)?|login|password|passwd|pwd|token|secret|api[_-]?key|access[_-]?token|client[_-]?secret|authorization|private[_-]?key)["']?\s*([:=])\s*(?:"[^"]*"|'[^']*'|[^,;\s}\]]+)"#,
            )
            .expect("valid store source redaction regex")
        })
        .replace_all(&value, "$1$2[已脱敏]")
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_is_independent_and_wal_enabled() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = AnalysisStore::open(dir.path())?;
        assert!(store.path().ends_with("reposcope.sqlite"));
        let connection = Connection::open(store.path())?;
        let mode: String = connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        assert_eq!(
            mode, "wal",
            "analysis database keeps concurrent readers available"
        );
        Ok(())
    }

    #[test]
    fn failed_run_does_not_replace_active_run() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = AnalysisStore::open(dir.path())?;
        let first = store.begin_run(Path::new("/repo"))?;
        store.finish_run(&first, AnalysisRunStatus::Failed, Some("x".into()))?;
        assert!(
            store.active_run()?.is_none(),
            "failed run is never activated"
        );
        Ok(())
    }

    #[test]
    fn progress_is_persisted_without_activating_a_run() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = AnalysisStore::open(dir.path())?;
        let run_id = store.begin_run(Path::new("/repo"))?;
        store.update_progress(&AnalysisStatus::running(
            &run_id,
            AnalysisPhase::Ownership,
            3,
            10,
        ))?;
        let status = store.status(Some(&run_id))?.expect("run status");
        assert_eq!(status.phase, AnalysisPhase::Ownership);
        assert_eq!(status.processed, 3);
        assert_eq!(status.total, 10);
        assert!((status.progress - 0.3).abs() < f64::EPSILON);
        assert!(store.active_run()?.is_none());
        Ok(())
    }

    #[test]
    fn terminal_progress_event_does_not_reopen_a_run() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = AnalysisStore::open(dir.path())?;
        let run_id = store.begin_run(Path::new("/repo"))?;
        store.finish_run(&run_id, AnalysisRunStatus::Complete, None)?;
        store.update_progress(&AnalysisStatus::finished(
            &run_id,
            AnalysisRunStatus::Complete,
        ))?;
        let status = store.run(&run_id)?.expect("run status");
        assert_eq!(status.status, AnalysisRunStatus::Complete);
        assert!(status.finished_at.is_some());
        Ok(())
    }

    #[test]
    fn terminal_error_is_redacted_before_persistence() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = AnalysisStore::open(dir.path())?;
        let run_id = store.begin_run(Path::new("/repo"))?;
        store.finish_run(
            &run_id,
            AnalysisRunStatus::Failed,
            Some(
                r#"https://alice:secret@example.com/api Bearer abc.def password="hunter2""#
                    .into(),
            ),
        )?;
        let error = store.run(&run_id)?.and_then(|run| run.error).unwrap();
        assert!(!error.contains("secret"));
        assert!(!error.contains("abc.def"));
        assert!(!error.contains("hunter2"));
        assert!(error.contains("[已脱敏]"));
        Ok(())
    }

    #[test]
    fn reopening_a_store_does_not_interrupt_a_live_run() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let first = AnalysisStore::open(dir.path())?;
        let run_id = first.begin_run(Path::new("/repo"))?;
        let second = AnalysisStore::open(dir.path())?;
        let status = second.status(Some(&run_id))?.expect("run status");
        assert_eq!(status.status, AnalysisRunStatus::Running);
        Ok(())
    }
}
