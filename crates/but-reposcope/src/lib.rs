//! Offline repository analysis for RepoScope Desktop.
//!
//! This crate deliberately has no dependency on Tauri, `but-api`, or the
//! desktop UI.  It reads a repository and its worktree, computes deterministic
//! reports, and stores each completed run in a separate SQLite database.  The
//! API and event layers can therefore be changed without changing the analysis
//! semantics.
#![deny(missing_docs)]

mod evidence;
mod history;
mod metrics;
mod model;
mod ownership;
mod store;
mod worktree;

pub use evidence::{EvidenceReports, analyze_evidence};
pub use history::{HistoryOptions, HistoryScanner, RepositoryFingerprint, compare_fingerprints};
pub use metrics::{
    CommitChange, CommitClassification, MetricAccumulator, classify_commit, compute_bus_factor,
    compute_couplings, compute_hotspot_score, compute_language_totals,
};
pub use model::{
    AnalysisConfig, AnalysisPhase, AnalysisRunStatus, AnalysisStatus, AuthorSummary,
    BusFactorReport, CodeAgeBucket, CommitDetail, CommitFileDetail, CommitSummary, CouplingSummary,
    DailyActivity, DirectoryOwnership, EvidenceItem, FileAgeStat, FileContent, FileSummary,
    Freshness, LanguageSummary, OverviewReport, OwnershipCoverage, OwnershipRow, Page,
    PathEncoding, RefSummary, ReportSnapshot, RepositoryDiagnostics, WorktreeSummary,
};
pub use ownership::{
    AGE_BUCKETS, BlameAdapter, BlameAttribution, OwnershipAnalysis, age_bucket, analyze_ownership,
    analyze_ownership_in_scope, tracked_paths,
};
pub use store::{AnalysisStore, StoredRun};
pub use worktree::{WorktreeFile, WorktreeScanOptions, scan_tree, scan_worktree};

use anyhow::{Context as _, Result};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::SystemTime;

/// A callback used by the desktop event bridge to publish progress.
pub type ProgressCallback = Arc<dyn Fn(AnalysisStatus) + Send + Sync + 'static>;

/// A cancellation token shared by the queue and the scanner.
#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    /// Create a token in the non-cancelled state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation.  The scanner observes this between commits and
    /// files, so cancellation never interrupts a SQLite transaction halfway.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Return whether cancellation was requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// The immutable result produced by one analysis pass.
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    /// The run identifier assigned by [`AnalysisStore`].
    pub run_id: String,
    /// High-level repository totals.
    pub overview: OverviewReport,
    /// Commit report rows, newest first.
    pub commits: Vec<CommitSummary>,
    /// First-parent file changes grouped by commit for drill-down queries.
    pub commit_files: Vec<(String, Vec<CommitChange>)>,
    /// File report rows, sorted by path.
    pub files: Vec<FileSummary>,
    /// Contributor report rows, sorted by contribution.
    pub authors: Vec<AuthorSummary>,
    /// Language totals.
    pub languages: Vec<LanguageSummary>,
    /// Daily activity, oldest first.
    pub activity: Vec<DailyActivity>,
    /// Ref tips visible to the analysis.
    pub refs: Vec<RefSummary>,
    /// Top co-change pairs.
    pub couplings: Vec<CouplingSummary>,
    /// Bus-factor result.
    pub bus_factor: BusFactorReport,
    /// Number of files in the real worktree included in the scan.
    pub worktree: WorktreeSummary,
    /// File-level blame ownership rows.
    pub ownership: Vec<OwnershipRow>,
    /// Directory-level blame ownership rows.
    pub directory_ownership: Vec<DirectoryOwnership>,
    /// Fixed code-age buckets.
    pub code_age: Vec<CodeAgeBucket>,
    /// Per-file code-age statistics.
    pub file_ages: Vec<FileAgeStat>,
    /// Coverage of the bounded blame pass.
    pub ownership_coverage: OwnershipCoverage,
    /// Whether the repository changed while this pass was running.
    pub freshness: Freshness,
    /// Fingerprint captured immediately before scanning.
    pub start_fingerprint: RepositoryFingerprint,
    /// Fingerprint captured immediately after scanning.
    pub end_fingerprint: RepositoryFingerprint,
    /// Delivery candidate evidence.
    pub delivery: ReportSnapshot,
    /// External-system footprint evidence.
    pub footprints: ReportSnapshot,
    /// Regional clue evidence.
    pub regions: ReportSnapshot,
    /// Dependency and plugin evidence.
    pub dependencies: ReportSnapshot,
    /// Whether one or more optional evidence stages failed while core
    /// reports were still usable.
    pub partial: bool,
}

/// A read-only engine that creates and activates complete analysis batches.
pub struct AnalysisEngine {
    store: AnalysisStore,
    config: AnalysisConfig,
    active_callbacks: Arc<Mutex<Vec<ProgressCallback>>>,
    current_run: Arc<Mutex<Option<String>>>,
}

impl AnalysisEngine {
    /// Open (or create) `reposcope.sqlite` below `data_dir`.
    pub fn open(data_dir: impl AsRef<Path>, config: AnalysisConfig) -> Result<Self> {
        Ok(Self {
            store: AnalysisStore::open(data_dir)?,
            config: config.normalized(),
            active_callbacks: Arc::new(Mutex::new(Vec::new())),
            current_run: Arc::new(Mutex::new(None)),
        })
    }

    /// Return the independent analysis store used by this engine.
    #[must_use]
    pub fn store(&self) -> &AnalysisStore {
        &self.store
    }

    /// Register a progress callback.  Callbacks are called synchronously from
    /// the scan thread and should only enqueue a UI event.
    pub fn subscribe(&self, callback: ProgressCallback) {
        self.active_callbacks.lock().push(callback);
    }

    /// Analyze a repository and atomically make the successful run active.
    ///
    /// A cancelled or failed pass leaves the previous active run untouched.
    pub fn analyze(
        &self,
        repository: impl Into<PathBuf>,
        cancellation: &CancellationToken,
    ) -> Result<AnalysisResult> {
        let repository = repository.into();
        let run_id = self.store.begin_run(&repository)?;
        *self.current_run.lock() = Some(run_id.clone());
        let started_at = SystemTime::now();
        self.emit(AnalysisStatus::running(
            &run_id,
            AnalysisPhase::History,
            0,
            0,
        ));

        let result = HistoryScanner::new(&repository, self.config.clone())
            .scan(cancellation, |status| self.emit(status))
            .with_context(|| format!("分析仓库失败: {}", repository.display()));
        match result {
            Ok(mut result) => {
                result.run_id.clone_from(&run_id);
                if cancellation.is_cancelled() {
                    let error = anyhow::anyhow!("分析已取消");
                    self.finish_terminal(
                        &run_id,
                        AnalysisRunStatus::Cancelled,
                        Some(error.to_string()),
                    );
                    return Err(error);
                }
                // Cancellation can arrive while the scanner is returning its
                // immutable result.  Check once more immediately before the
                // activation transaction so a superseded run cannot replace a
                // newer project's active snapshot.
                if cancellation.is_cancelled() {
                    let error = anyhow::anyhow!("分析已取消");
                    self.finish_terminal(
                        &run_id,
                        AnalysisRunStatus::Cancelled,
                        Some(error.to_string()),
                    );
                    return Err(error);
                }
                if let Err(error) = self.store.activate_result(&run_id, &result, started_at) {
                    let error = error.context("保存 RepoScope 分析结果失败");
                    self.finish_terminal(
                        &run_id,
                        AnalysisRunStatus::Failed,
                        Some(error.to_string()),
                    );
                    return Err(error);
                }
                let terminal_status = if result.partial {
                    AnalysisRunStatus::Partial
                } else {
                    AnalysisRunStatus::Complete
                };
                self.emit(AnalysisStatus::finished(&run_id, terminal_status));
                *self.current_run.lock() = None;
                Ok(result)
            }
            Err(error) => {
                let status = if cancellation.is_cancelled() {
                    AnalysisRunStatus::Cancelled
                } else {
                    AnalysisRunStatus::Failed
                };
                self.finish_terminal(&run_id, status, Some(error.to_string()));
                Err(error)
            }
        }
    }

    fn finish_terminal(
        &self,
        run_id: &str,
        status: AnalysisRunStatus,
        error: Option<String>,
    ) {
        let safe_error = error.map(|value| store::sanitize_error(&value));
        if let Err(finish_error) = self
            .store
            .finish_run(run_id, status, safe_error.clone())
        {
            // The scan result is already discarded on this path.  Clear the
            // in-memory run even when SQLite itself is unavailable, otherwise
            // a later retry would publish progress under a stale run id.
            tracing::error!(?finish_error, %run_id, "无法保存 RepoScope 分析终态");
        }
        let mut terminal = AnalysisStatus::finished(run_id, status);
        terminal.error = safe_error;
        self.emit(terminal);
        *self.current_run.lock() = None;
    }

    fn emit(&self, status: AnalysisStatus) {
        let mut status = status;
        if status.run_id.is_none() || status.run_id.as_deref() == Some("") {
            status.run_id = self.current_run.lock().clone();
        }
        if let Err(error) = self.store.update_progress(&status) {
            // Progress persistence is best-effort.  The event stream remains
            // authoritative for the live UI, and a failed status write must
            // never turn a read-only analysis into a failed batch.
            tracing::debug!(?error, "无法持久化 RepoScope 分析进度");
        }
        for callback in self.active_callbacks.lock().iter() {
            callback(status.clone());
        }
    }
}
