//! RepoScope Desktop's offline analysis API.
//!
//! The functions in this module are intentionally thin: all Git traversal,
//! limits and persistence live in [`but_reposcope`].  This keeps the API usable
//! by Tauri and N-API callers without introducing a second analysis path.

use but_api_macros::but_api;
use but_ctx::Context;
use but_reposcope::{
    AnalysisConfig, AnalysisEngine, AnalysisRunStatus, AnalysisStatus, AnalysisStore,
    AuthorSummary, BusFactorReport, CodeAgeBucket, CommitDetail, CommitSummary, CouplingSummary,
    DirectoryOwnership, FileContent, FileSummary, LanguageSummary, OwnershipCoverage, OwnershipRow,
    Page, RefSummary, ReportSnapshot, RepositoryDiagnostics, RepositoryFingerprint,
};
use parking_lot::Mutex;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};

const FILE_CONTENT_LIMIT: usize = 1_500_000;
const MAX_SOURCE_FILE_BYTES: usize = 20_000_000;
const ANALYSIS_SETTINGS_FILE: &str = "reposcope-settings.json";

#[derive(Clone)]
struct AnalysisJob {
    token: but_reposcope::CancellationToken,
    status: Arc<Mutex<AnalysisStatus>>,
}

static JOBS: OnceLock<Mutex<HashMap<PathBuf, AnalysisJob>>> = OnceLock::new();
type RepoScopeEventCallback = Arc<dyn Fn(String, AnalysisStatus) + Send + Sync + 'static>;
static EVENT_CALLBACKS: OnceLock<Mutex<Vec<RepoScopeEventCallback>>> = OnceLock::new();

fn jobs() -> &'static Mutex<HashMap<PathBuf, AnalysisJob>> {
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn event_callbacks() -> &'static Mutex<Vec<RepoScopeEventCallback>> {
    EVENT_CALLBACKS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Register a process-local sink for analysis progress events.
///
/// The Tauri host installs one sink during setup and forwards updates to the
/// `project://{projectId}/reposcope-analysis` event channel.  Keeping this
/// bridge outside the API macro means N-API callers can still use the same
/// analysis implementation without depending on a Tauri application handle.
pub fn subscribe_reposcope_events(
    callback: impl Fn(String, AnalysisStatus) + Send + Sync + 'static,
) {
    event_callbacks().lock().push(Arc::new(callback));
}

fn publish_event(project_id: &str, status: &AnalysisStatus) {
    let callbacks = event_callbacks().lock().clone();
    for callback in callbacks {
        callback(project_id.to_owned(), status.clone());
    }
}

fn event_project_id(ctx: &Context) -> String {
    #[cfg(feature = "legacy")]
    {
        ctx.legacy_project.id.to_string()
    }
    #[cfg(not(feature = "legacy"))]
    {
        // A non-legacy API caller has no UUID project handle.  The data
        // directory is stable for the lifetime of the project and is only a
        // fallback; Tauri builds always use the legacy UUID above.
        ctx.project_data_dir().to_string_lossy().into_owned()
    }
}

/// Configuration accepted by the frontend when starting a scan.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct AnalysisConfigInput {
    /// Maximum worktree file size in bytes.
    pub max_file_bytes: Option<u64>,
    /// Maximum files considered by future blame ownership stages.
    pub ownership_max_files: Option<usize>,
    /// Worker count.
    pub workers: Option<usize>,
    /// Coupling commit file limit.
    pub coupling_max_files: Option<usize>,
    /// Coupling row limit.
    pub coupling_top_pairs: Option<usize>,
}

#[cfg(feature = "export-schema")]
but_schemars::register_sdk_type!(AnalysisConfigInput);

impl From<AnalysisConfigInput> for AnalysisConfig {
    fn from(input: AnalysisConfigInput) -> Self {
        let defaults = Self::default();
        Self {
            max_file_bytes: input
                .max_file_bytes
                .unwrap_or(defaults.max_file_bytes)
                // The compatibility contract intentionally caps a single
                // source file at 20 MB.  A per-project setting may lower the
                // limit, but it must never widen the reader's safety bound.
                .clamp(1, 20_000_000),
            ownership_max_files: input
                .ownership_max_files
                .unwrap_or(defaults.ownership_max_files)
                .clamp(1, 100_000),
            workers: input.workers.unwrap_or(defaults.workers).clamp(1, 8),
            coupling_max_files: input
                .coupling_max_files
                .unwrap_or(defaults.coupling_max_files)
                .clamp(2, 200),
            coupling_top_pairs: input
                .coupling_top_pairs
                .unwrap_or(defaults.coupling_top_pairs)
                .clamp(1, 5_000),
        }
        .normalized()
    }
}

/// Queue a new read-only analysis.  A running analysis for the same project is
/// cancelled first, and its previous active batch remains available.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_analysis_start(
    ctx: &Context,
    config: Option<AnalysisConfigInput>,
) -> anyhow::Result<AnalysisStatus> {
    let project_dir = ctx.project_data_dir();
    let project_id = event_project_id(ctx);
    if let Some(previous) = jobs().lock().get(&project_dir) {
        previous.token.cancel();
    }
    let token = but_reposcope::CancellationToken::new();
    let status = Arc::new(Mutex::new(AnalysisStatus {
        run_id: None,
        status: AnalysisRunStatus::Queued,
        phase: but_reposcope::AnalysisPhase::Queued,
        progress: 0.0,
        processed: 0,
        total: 0,
        error: None,
        freshness: None,
    }));
    let job = AnalysisJob {
        token: token.clone(),
        status: status.clone(),
    };
    jobs().lock().insert(project_dir.clone(), job);
    let queued_status = status.lock().clone();
    publish_event(&project_id, &queued_status);
    let repository = match ctx.workdir_or_gitdir() {
        Ok(repository) => repository,
        Err(error) => {
            let failed = AnalysisStatus {
                run_id: None,
                status: AnalysisRunStatus::Failed,
                phase: but_reposcope::AnalysisPhase::Persisting,
                progress: 1.0,
                processed: 0,
                total: 0,
                error: Some(safe_error(&error)),
                freshness: None,
            };
            *status.lock() = failed.clone();
            publish_event(&project_id, &failed);
            remove_job_if_current(&project_dir, &status);
            return Err(error);
        }
    };
    let config = config.map(Into::into).unwrap_or_else(|| {
        load_analysis_config(&project_dir).unwrap_or_else(|error| {
            tracing::warn!(?error, "读取 RepoScope 分析设置失败，使用默认值");
            AnalysisConfig::default()
        })
    });
    let thread_status = status.clone();
    let thread_project_dir = project_dir.clone();
    let thread_project_id = project_id.clone();
    let spawn_result = std::thread::Builder::new()
        .name("reposcope-analysis".to_owned())
        .spawn(move || {
            let engine = match AnalysisEngine::open(&thread_project_dir, config) {
                Ok(engine) => engine,
                Err(error) => {
                    let failed = AnalysisStatus {
                        run_id: None,
                        status: AnalysisRunStatus::Failed,
                        phase: but_reposcope::AnalysisPhase::Persisting,
                        progress: 1.0,
                        processed: 0,
                        total: 0,
                        error: Some(safe_error(&error)),
                        freshness: None,
                    };
                    *thread_status.lock() = failed.clone();
                    publish_event(&thread_project_id, &failed);
                    remove_job_if_current(&thread_project_dir, &thread_status);
                    return;
                }
            };
            let status_for_callback = thread_status.clone();
            let project_id_for_callback = thread_project_id.clone();
            engine.subscribe(Arc::new(move |update| {
                let mut current = status_for_callback.lock();
                // Keep an explicit user cancellation visible while the scan
                // finishes its current file/commit.  The engine's terminal
                // event is still accepted below, but a late running update
                // must not make a cancelled job look active again.
                if current.status == AnalysisRunStatus::Cancelled
                    && update.status == AnalysisRunStatus::Running
                {
                    return;
                }
                *current = update.clone();
                drop(current);
                publish_event(&project_id_for_callback, &update);
            }));
            match engine.analyze(repository, &token) {
                Ok(result) => {
                    // The engine emits a generic terminal event before it
                    // returns.  Re-read the committed row here so the final
                    // event also carries the persisted freshness and the
                    // actual `partial` status.
                    if let Ok(Some(saved)) = engine.store().status(Some(&result.run_id)) {
                        *thread_status.lock() = saved.clone();
                        publish_event(&thread_project_id, &saved);
                    }
                }
                Err(error) => {
                    let mut current = thread_status.lock();
                    current.status = if token.is_cancelled() {
                        AnalysisRunStatus::Cancelled
                    } else {
                        AnalysisRunStatus::Failed
                    };
                    current.error = Some(safe_error(&error));
                    current.progress = 1.0;
                    let terminal = current.clone();
                    drop(current);
                    publish_event(&thread_project_id, &terminal);
                }
            }
            remove_job_if_current(&thread_project_dir, &thread_status);
        });
    if let Err(error) = spawn_result {
        let failed = AnalysisStatus {
            run_id: None,
            status: AnalysisRunStatus::Failed,
            phase: but_reposcope::AnalysisPhase::Persisting,
            progress: 1.0,
            processed: 0,
            total: 0,
            error: Some(format!("启动 RepoScope 分析线程失败：{error}")),
            freshness: None,
        };
        *status.lock() = failed.clone();
        publish_event(&project_id, &failed);
        remove_job_if_current(&project_dir, &status);
        return Err(anyhow::anyhow!("启动 RepoScope 分析线程失败: {error}"));
    }
    Ok(status.lock().clone())
}

fn remove_job_if_current(project_dir: &Path, status: &Arc<Mutex<AnalysisStatus>>) {
    let mut jobs = jobs().lock();
    let is_current = jobs
        .get(project_dir)
        .is_some_and(|job| Arc::ptr_eq(&job.status, status));
    if is_current {
        jobs.remove(project_dir);
    }
}

/// Request cancellation of the current project analysis.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_analysis_cancel(ctx: &Context) -> anyhow::Result<AnalysisStatus> {
    let project_dir = ctx.project_data_dir();
    if let Some(job) = jobs().lock().get(&project_dir) {
        job.token.cancel();
        let mut status = job.status.lock();
        if matches!(
            status.status,
            AnalysisRunStatus::Queued | AnalysisRunStatus::Running
        ) {
            status.status = AnalysisRunStatus::Cancelled;
        }
        let cancelled = status.clone();
        drop(status);
        publish_event(&event_project_id(ctx), &cancelled);
        return Ok(cancelled);
    }
    let status = AnalysisStatus {
        run_id: None,
        status: AnalysisRunStatus::Cancelled,
        phase: but_reposcope::AnalysisPhase::Queued,
        progress: 0.0,
        processed: 0,
        total: 0,
        error: None,
        freshness: None,
    };
    publish_event(&event_project_id(ctx), &status);
    Ok(status)
}

/// Read live progress or the last active run status.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_analysis_status(ctx: &Context) -> anyhow::Result<Option<AnalysisStatus>> {
    let project_dir = ctx.project_data_dir();
    if let Some(job) = jobs().lock().get(&project_dir) {
        return Ok(Some(job.status.lock().clone()));
    }
    let store = AnalysisStore::open(&project_dir)?;
    let mut status = store.status(None)?;
    let active = store.active_run()?;
    if let Some(active) = active
        && let (Some(status), Some(end_refs), Ok(location)) =
            (status.as_mut(), active.end_refs.as_deref(), ctx.workdir_or_gitdir())
        && let Ok(current) = RepositoryFingerprint::for_repository(location)
    {
        let last = RepositoryFingerprint {
            refs: end_refs.to_owned(),
            worktree: active.end_worktree.clone(),
        };
        // A failed/cancelled retry can be newer than the active successful
        // batch.  Keep the retry status visible, but still report whether the
        // last usable result is stale relative to the current repository.
        status.freshness = Some(but_reposcope::compare_fingerprints(&last, &current));
    }
    Ok(status)
}

/// Read the persisted per-project analysis settings.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_analysis_config(ctx: &Context) -> anyhow::Result<AnalysisConfig> {
    load_analysis_config(&ctx.project_data_dir())
}

/// Persist per-project analysis settings for subsequent automatic scans.
#[but_api(napi, invalidates = [RepoScopeAnalysis])]
pub fn reposcope_analysis_config_set(
    ctx: &Context,
    config: AnalysisConfigInput,
) -> anyhow::Result<AnalysisConfig> {
    let config: AnalysisConfig = config.into();
    save_analysis_config(&ctx.project_data_dir(), &config)?;
    Ok(config)
}

/// Return high-level repository totals from the active analysis batch.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_overview(ctx: &Context) -> anyhow::Result<Option<but_reposcope::OverviewReport>> {
    snapshot(ctx, "overview")
}

/// Return daily UTC activity.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_activity(ctx: &Context) -> anyhow::Result<Vec<but_reposcope::DailyActivity>> {
    Ok(snapshot::<Vec<but_reposcope::DailyActivity>>(ctx, "activity")?.unwrap_or_default())
}

/// Return paginated commit summaries.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_commits(
    ctx: &Context,
    page: Option<u32>,
    page_size: Option<u32>,
    query: Option<String>,
) -> anyhow::Result<Page<CommitSummary>> {
    let rows: Vec<CommitSummary> = snapshot(ctx, "commits")?.unwrap_or_default();
    let query = query.unwrap_or_default().to_ascii_lowercase();
    paginate(
        rows.into_iter().filter(|row| {
            query.is_empty()
                || row.subject.to_ascii_lowercase().contains(&query)
                || row.author_name.to_ascii_lowercase().contains(&query)
                || row.oid.starts_with(&query)
        }),
        page,
        page_size,
    )
}

/// Return paginated file evolution summaries.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_files(
    ctx: &Context,
    page: Option<u32>,
    page_size: Option<u32>,
    query: Option<String>,
) -> anyhow::Result<Page<FileSummary>> {
    let rows: Vec<FileSummary> = snapshot(ctx, "files")?.unwrap_or_default();
    let query = query.unwrap_or_default().to_ascii_lowercase();
    paginate(
        rows.into_iter()
            .filter(|row| query.is_empty() || row.path.to_ascii_lowercase().contains(&query)),
        page,
        page_size,
    )
}

/// Return paginated contributor summaries.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_authors(
    ctx: &Context,
    page: Option<u32>,
    page_size: Option<u32>,
    query: Option<String>,
) -> anyhow::Result<Page<AuthorSummary>> {
    let rows: Vec<AuthorSummary> = snapshot(ctx, "authors")?.unwrap_or_default();
    let query = query.unwrap_or_default().to_ascii_lowercase();
    paginate(
        rows.into_iter().filter(|row| {
            query.is_empty()
                || row.name.to_ascii_lowercase().contains(&query)
                || row.email.to_ascii_lowercase().contains(&query)
        }),
        page,
        page_size,
    )
}

/// Return language totals.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_languages(ctx: &Context) -> anyhow::Result<Vec<LanguageSummary>> {
    Ok(snapshot::<Vec<LanguageSummary>>(ctx, "languages")?.unwrap_or_default())
}

/// Return one commit summary by full or abbreviated object id.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_commit_detail(ctx: &Context, oid: String) -> anyhow::Result<Option<CommitDetail>> {
    let rows: Vec<CommitSummary> = snapshot(ctx, "commits")?.unwrap_or_default();
    let Some(commit) = rows
        .into_iter()
        .find(|row| row.oid == oid || row.short_oid == oid)
    else {
        return Ok(None);
    };
    let files = AnalysisStore::open(ctx.project_data_dir())?.active_commit_files(&commit.oid)?;
    Ok(Some(CommitDetail { commit, files }))
}

/// Return one file summary by relative path.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_file_detail(ctx: &Context, path: String) -> anyhow::Result<Option<FileSummary>> {
    let rows: Vec<FileSummary> = snapshot(ctx, "files")?.unwrap_or_default();
    Ok(rows.into_iter().find(|row| row.path == path))
}

/// Read a validated, redacted source file from the current worktree or a
/// historical commit tree.  Historical reads are object-store only and never
/// materialize a checkout or mutate the GitButler workspace.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_file_content(
    ctx: &Context,
    path: String,
    line: Option<u64>,
    revision: Option<String>,
) -> anyhow::Result<FileContent> {
    let relative = validate_relative_path(&path)?;
    let display_path = relative.to_string_lossy().replace('\\', "/");
    let revision = revision.filter(|value| !value.trim().is_empty());
    let (bytes, resolved_revision) = match revision {
        Some(revision) => {
            let location = ctx.workdir_or_gitdir()?;
            let repo = gix::discover(&location)?;
            let commit_id = resolve_commit_id(ctx, &repo, revision.trim())?;
            let commit = repo.find_commit(commit_id)?;
            let tree = commit.tree()?;
            let entry = tree
                .lookup_entry_by_path(display_path.as_str())?
                .ok_or_else(|| anyhow::anyhow!("该提交中不存在此文件"))?;
            if entry.mode().is_tree() || entry.mode().is_link() {
                anyhow::bail!("历史对象不是普通文件");
            }
            let blob = repo.find_blob(entry.id().detach())?;
            ensure_source_size(blob.data.len())?;
            (blob.data.to_vec(), Some(commit_id.to_hex().to_string()))
        }
        None => {
            let location = ctx.workdir_or_gitdir()?;
            let repo = gix::discover(&location)?;
            if let Some(file) = ctx
                .workdir()?
                .map(|_| safe_worktree_file(ctx, &display_path))
            {
                let file = file?;
                (
                    read_source_file(&file)?,
                    None,
                )
            } else {
                let tree_id = repo
                    .head_tree_id()
                    .map_err(|error| anyhow::anyhow!("裸仓库没有可预览的 HEAD: {error}"))?;
                let tree = repo.find_tree(tree_id.detach())?;
                let entry = tree
                    .lookup_entry_by_path(display_path.as_str())?
                    .ok_or_else(|| anyhow::anyhow!("当前版本中不存在此文件"))?;
                if entry.mode().is_tree() || entry.mode().is_link() {
                    anyhow::bail!("当前版本对象不是普通文件");
                }
                let blob = repo.find_blob(entry.id().detach())?;
                ensure_source_size(blob.data.len())?;
                let revision = repo
                    .head_commit()
                    .ok()
                    .map(|commit| commit.id.to_hex().to_string());
                (blob.data.to_vec(), revision)
            }
        }
    };
    if bytes.iter().take(8_192).any(|byte| *byte == 0) {
        anyhow::bail!("二进制文件不支持源码预览");
    }
    let truncated = bytes.len() > FILE_CONTENT_LIMIT;
    let content = String::from_utf8(bytes[..bytes.len().min(FILE_CONTENT_LIMIT)].to_vec())
        .map_err(|_| anyhow::anyhow!("文件不是 UTF-8 文本"))?;
    let content = redact_source(&content);
    let total_lines = content.lines().count() as u64;
    let highlight_line = line.unwrap_or(1).max(1).min(total_lines.max(1));
    Ok(FileContent {
        path: display_path,
        revision: resolved_revision,
        language: detect_language_for_path(&path),
        content,
        total_lines,
        highlight_line,
        truncated,
    })
}

fn resolve_commit_id(
    ctx: &Context,
    repo: &gix::Repository,
    revision: &str,
) -> anyhow::Result<gix::ObjectId> {
    if let Ok(id) = gix::ObjectId::from_hex(revision.as_bytes())
        && repo.find_commit(id).is_ok()
    {
        return Ok(id);
    }
    let rows: Vec<CommitSummary> = snapshot(ctx, "commits")?.unwrap_or_default();
    rows.into_iter()
        .find(|row| row.oid == revision || row.short_oid == revision)
        .and_then(|row| gix::ObjectId::from_hex(row.oid.as_bytes()).ok())
        .filter(|id| repo.find_commit(*id).is_ok())
        .ok_or_else(|| anyhow::anyhow!("提交 OID 无效或不属于当前仓库"))
}

fn detect_language_for_path(path: &str) -> Option<String> {
    let basename = path.rsplit('/').next().unwrap_or(path);
    let extension = basename.rsplit_once('.').map(|(_, extension)| extension);
    let language = match extension.map(str::to_ascii_lowercase).as_deref() {
        Some("rs") => Some("Rust"),
        Some("ts" | "tsx") => Some("TypeScript"),
        Some("js" | "jsx" | "mjs" | "cjs") => Some("JavaScript"),
        Some("py" | "pyi") => Some("Python"),
        Some("java") => Some("Java"),
        Some("go") => Some("Go"),
        Some("json") => Some("JSON"),
        Some("yaml" | "yml") => Some("YAML"),
        Some("toml") => Some("TOML"),
        Some("md" | "mdx") => Some("Markdown"),
        Some("css") => Some("CSS"),
        Some("html" | "htm") => Some("HTML"),
        Some("sql") => Some("SQL"),
        _ => None,
    };
    language.map(str::to_owned)
}

/// Return files ordered by the compatibility hotspot score.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_hotspots(
    ctx: &Context,
    page: Option<u32>,
    page_size: Option<u32>,
) -> anyhow::Result<Page<FileSummary>> {
    let mut rows: Vec<FileSummary> = snapshot(ctx, "files")?.unwrap_or_default();
    rows.sort_by(|left, right| right.hotspot_score.total_cmp(&left.hotspot_score));
    paginate(rows, page, page_size)
}

/// Return paginated co-change pairs.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_coupling(
    ctx: &Context,
    page: Option<u32>,
    page_size: Option<u32>,
) -> anyhow::Result<Page<CouplingSummary>> {
    let rows: Vec<CouplingSummary> = snapshot(ctx, "couplings")?.unwrap_or_default();
    paginate(rows, page, page_size)
}

/// Return file-level ownership rows when the blame stage has populated them.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_ownership(
    ctx: &Context,
    page: Option<u32>,
    page_size: Option<u32>,
) -> anyhow::Result<Page<OwnershipRow>> {
    paginate(
        snapshot::<Vec<OwnershipRow>>(ctx, "ownership")?.unwrap_or_default(),
        page,
        page_size,
    )
}

/// Return the explicit coverage of the bounded blame pass.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_ownership_coverage(ctx: &Context) -> anyhow::Result<OwnershipCoverage> {
    Ok(snapshot::<OwnershipCoverage>(ctx, "ownershipCoverage")?.unwrap_or_default())
}

/// Return per-file code-age statistics for the active batch.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_file_age_stats(
    ctx: &Context,
    page: Option<u32>,
    page_size: Option<u32>,
) -> anyhow::Result<Page<but_reposcope::FileAgeStat>> {
    paginate(
        snapshot::<Vec<but_reposcope::FileAgeStat>>(ctx, "fileAgeStats")?.unwrap_or_default(),
        page,
        page_size,
    )
}

/// Return directory ownership totals.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_directory_ownership(
    ctx: &Context,
    page: Option<u32>,
    page_size: Option<u32>,
) -> anyhow::Result<Page<DirectoryOwnership>> {
    paginate(
        snapshot::<Vec<DirectoryOwnership>>(ctx, "directoryOwnership")?.unwrap_or_default(),
        page,
        page_size,
    )
}

/// Return code-age buckets.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_code_age(ctx: &Context) -> anyhow::Result<Vec<CodeAgeBucket>> {
    Ok(snapshot::<Vec<CodeAgeBucket>>(ctx, "codeAge")?.unwrap_or_default())
}

/// Return the LOC and commit Bus Factor.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_bus_factor(ctx: &Context) -> anyhow::Result<BusFactorReport> {
    Ok(snapshot::<BusFactorReport>(ctx, "busFactor")?.unwrap_or_default())
}

/// Return the delivery-forensics report, if the optional stage completed.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_delivery(ctx: &Context) -> anyhow::Result<ReportSnapshot> {
    report_snapshot(ctx, "delivery")
}

/// Return external-system footprint evidence, if available.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_footprints(ctx: &Context) -> anyhow::Result<ReportSnapshot> {
    report_snapshot(ctx, "footprints")
}

/// Return cautious regional clues, if available.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_regions(ctx: &Context) -> anyhow::Result<ReportSnapshot> {
    report_snapshot(ctx, "regions")
}

/// Return dependency and plugin evidence, if available.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_dependencies(ctx: &Context) -> anyhow::Result<ReportSnapshot> {
    report_snapshot(ctx, "dependencies")
}

/// Return read-only repository diagnostics.  Hooks are listed but never run.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_repository_diagnostics(ctx: &Context) -> anyhow::Result<RepositoryDiagnostics> {
    let location = ctx.workdir_or_gitdir()?;
    let repo = gix::discover(&location)?;
    let git_dir = repo.git_dir().to_owned();
    let refs = repo
        .references()?
        .all()?
        .filter_map(Result::ok)
        .filter(|reference| !is_internal_ref(reference.name().as_bstr()))
        .count() as u64;
    let worktree = repo
        .workdir()
        .map(|path| path.to_string_lossy().to_string());
    let hook_dir = git_dir.join("hooks");
    let hooks = std::fs::read_dir(&hook_dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
        })
        .count() as u64;
    let worktrees = git_dir
        .join("worktrees")
        .read_dir()
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .count() as u64;
    let submodules = repo
        .workdir()
        .is_some_and(|path| path.join(".gitmodules").is_file());
    let lfs = git_dir.join("lfs").exists()
        || repo.workdir().is_some_and(|path| {
            std::fs::read_to_string(path.join(".gitattributes"))
                .map(|value| value.contains("filter=lfs"))
                .unwrap_or(false)
        });
    Ok(RepositoryDiagnostics {
        bare: repo.workdir().is_none(),
        worktree,
        refs,
        stash: repo.find_reference("refs/stash").is_ok(),
        reflog: git_dir.join("logs").join("HEAD").is_file(),
        worktrees,
        submodules,
        hooks,
        lfs,
        shallow: git_dir.join("shallow").is_file(),
    })
}

fn is_internal_ref(name: &bstr::BStr) -> bool {
    name.starts_with(b"refs/gitbutler/")
        || name.starts_with(b"refs/heads/gitbutler/")
        || name.starts_with(b"refs/remotes/gitbutler/")
        || name.starts_with(b"refs/namespaces/gitbutler-stashes/")
        || name.starts_with(b"refs/namespaces/gitbutler/")
}

/// Return visible refs from the active batch.
#[but_api(napi, provides = [RepoScopeAnalysis])]
pub fn reposcope_refs(
    ctx: &Context,
    page: Option<u32>,
    page_size: Option<u32>,
) -> anyhow::Result<Page<RefSummary>> {
    paginate(
        snapshot::<Vec<RefSummary>>(ctx, "refs")?.unwrap_or_default(),
        page,
        page_size,
    )
}

fn snapshot<T: serde::de::DeserializeOwned>(
    ctx: &Context,
    kind: &str,
) -> anyhow::Result<Option<T>> {
    AnalysisStore::open(ctx.project_data_dir())?.active_snapshot(kind)
}

fn load_analysis_config(data_dir: &Path) -> anyhow::Result<AnalysisConfig> {
    let path = data_dir.join(ANALYSIS_SETTINGS_FILE);
    match std::fs::read_to_string(&path) {
        Ok(contents) => Ok(serde_json::from_str::<AnalysisConfig>(&contents)
            .map_err(|error| anyhow::anyhow!("解析 RepoScope 分析设置失败: {error}"))?
            .normalized()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AnalysisConfig::default()),
        Err(error) => Err(anyhow::anyhow!("读取 RepoScope 分析设置失败: {error}")),
    }
}

fn save_analysis_config(data_dir: &Path, config: &AnalysisConfig) -> anyhow::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join(ANALYSIS_SETTINGS_FILE);
    let temporary = data_dir.join(format!("{ANALYSIS_SETTINGS_FILE}.tmp"));
    let payload = serde_json::to_vec_pretty(config)?;
    std::fs::write(&temporary, payload)?;
    // Keep the write recoverable on process interruption.  Windows does not
    // replace an existing destination with `rename`, so remove only this
    // narrow settings file before the final move.
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    std::fs::rename(&temporary, &path)?;
    Ok(())
}

fn report_snapshot(ctx: &Context, kind: &str) -> anyhow::Result<ReportSnapshot> {
    if let Some(snapshot) = snapshot::<ReportSnapshot>(ctx, kind)? {
        return Ok(snapshot);
    }
    Ok(ReportSnapshot {
        kind: kind.to_owned(),
        available: false,
        items: Vec::new(),
        limitations: vec!["该可选报告尚未完成迁移或本次分析未启用。".to_owned()],
        score: None,
        level: None,
        conclusion: None,
        scanned_files: 0,
        truncated: false,
    })
}

fn paginate<T>(
    rows: impl IntoIterator<Item = T>,
    page: Option<u32>,
    page_size: Option<u32>,
) -> anyhow::Result<Page<T>> {
    let rows: Vec<T> = rows.into_iter().collect();
    let total = rows.len() as u64;
    let (page, page_size) = Page::<T>::normalize(page.unwrap_or(1), page_size.unwrap_or(50));
    let start = (page as usize - 1).saturating_mul(page_size as usize);
    let items = rows
        .into_iter()
        .skip(start)
        .take(page_size as usize)
        .collect();
    Ok(Page {
        items,
        total,
        page,
        page_size,
    })
}

fn safe_worktree_file(ctx: &Context, path: &str) -> anyhow::Result<PathBuf> {
    let worktree = ctx
        .workdir()?
        .ok_or_else(|| anyhow::anyhow!("裸仓库没有可预览的工作目录"))?;
    let relative = validate_relative_path(path)?;
    let root = worktree.canonicalize()?;
    reject_symlink_components(&root, &relative)?;
    let candidate = root.join(&relative);
    let metadata = std::fs::symlink_metadata(&candidate)
        .map_err(|error| anyhow::anyhow!("文件不存在: {error}"))?;
    if metadata.file_type().is_symlink() {
        anyhow::bail!("不允许读取符号链接");
    }
    let canonical = candidate.canonicalize()?;
    if !canonical.starts_with(&root) {
        anyhow::bail!("文件路径超出仓库边界");
    }
    if !metadata.is_file() {
        anyhow::bail!("预览目标不是普通文件");
    }
    ensure_source_size(metadata.len() as usize)?;
    Ok(canonical)
}

fn ensure_source_size(bytes: usize) -> anyhow::Result<()> {
    if bytes > MAX_SOURCE_FILE_BYTES {
        anyhow::bail!("文件超过 20 MB，拒绝源码预览");
    }
    Ok(())
}

fn read_source_file(path: &Path) -> anyhow::Result<Vec<u8>> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| anyhow::anyhow!("读取文件失败: {error}"))?;
    let mut bytes = Vec::with_capacity(MAX_SOURCE_FILE_BYTES.saturating_add(1));
    file.by_ref()
        .take(MAX_SOURCE_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| anyhow::anyhow!("读取文件失败: {error}"))?;
    ensure_source_size(bytes.len())?;
    Ok(bytes)
}

fn reject_symlink_components(root: &Path, relative: &Path) -> anyhow::Result<()> {
    let mut current = root.to_owned();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        current.push(part);
        if let Ok(metadata) = std::fs::symlink_metadata(&current)
            && metadata.file_type().is_symlink()
        {
            anyhow::bail!("不允许读取符号链接");
        }
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> anyhow::Result<PathBuf> {
    if path.is_empty() || path.contains('\0') {
        anyhow::bail!("文件路径为空或包含非法字符");
    }
    let normalized = path.replace('\\', "/");
    let mut output = PathBuf::new();
    for component in Path::new(&normalized).components() {
        match component {
            Component::Normal(part) => output.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("文件路径必须位于仓库内");
            }
        }
    }
    if output.as_os_str().is_empty() {
        anyhow::bail!("文件路径为空");
    }
    Ok(output)
}

fn redact_source(content: &str) -> String {
    let value = content.replace('\0', " ");
    static URL_USERINFO: OnceLock<Regex> = OnceLock::new();
    static JDBC_USERINFO: OnceLock<Regex> = OnceLock::new();
    static BEARER: OnceLock<Regex> = OnceLock::new();
    static SECRET: OnceLock<Regex> = OnceLock::new();
    let value = URL_USERINFO
        .get_or_init(|| {
            Regex::new(r#"(?i)(\b[a-z][a-z0-9+.-]*://)[^/\s:@]+(?::[^/\s@]*)?@"#)
                .expect("valid source URL credential regex")
        })
        .replace_all(&value, "$1[已脱敏]@");
    let value = JDBC_USERINFO
        .get_or_init(|| {
            Regex::new(r#"(?i)(\bjdbc:[^:\s]+(?::[^:\s]+)*:)[^/\s:@]+/[^@\s]+@"#)
                .expect("valid source JDBC credential regex")
        })
        .replace_all(&value, "$1[已脱敏]@");
    let value = BEARER
        .get_or_init(|| {
            Regex::new(r#"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+"#)
                .expect("valid source bearer credential regex")
        })
        .replace_all(&value, "Bearer [已脱敏]");
    SECRET
        .get_or_init(|| {
            Regex::new(
                r#"(?i)["']?(user(?:name)?|login|password|passwd|pwd|token|secret|api[_-]?key|access[_-]?token|client[_-]?secret|authorization|private[_-]?key)["']?\s*([:=])\s*(?:"[^"]*"|'[^']*'|[^,;\s}\]]+)"#,
            )
            .expect("valid source redaction regex")
        })
        .replace_all(&value, "$1$2[已脱敏]")
        .into_owned()
}

fn safe_error(error: &anyhow::Error) -> String {
    redact_source(&error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_preview_rejects_paths_outside_repository() {
        for path in ["../secret.txt", "a/../../secret.txt", "/absolute.txt", ""] {
            assert!(validate_relative_path(path).is_err(), "path should be rejected: {path:?}");
        }
        assert_eq!(
            validate_relative_path(r"src\main.rs").unwrap(),
            PathBuf::from("src/main.rs")
        );
    }

    #[test]
    fn file_preview_enforces_input_limit_before_reading() {
        assert!(ensure_source_size(MAX_SOURCE_FILE_BYTES).is_ok());
        assert!(ensure_source_size(MAX_SOURCE_FILE_BYTES + 1).is_err());
    }

    #[test]
    fn analysis_config_cannot_widen_the_20_mb_reader_limit() {
        let config = AnalysisConfigInput {
            max_file_bytes: Some(100_000_000),
            ..AnalysisConfigInput::default()
        };
        assert_eq!(AnalysisConfig::from(config).max_file_bytes, MAX_SOURCE_FILE_BYTES as u64);
    }

    #[test]
    fn errors_and_source_are_redacted_before_transport() {
        let value = redact_source(
            r#"https://alice:secret@example.com/api Bearer abc.def password="hunter2""#,
        );
        assert!(!value.contains("secret"));
        assert!(!value.contains("abc.def"));
        assert!(!value.contains("hunter2"));
        assert!(safe_error(&anyhow::anyhow!("token: raw-secret")).contains("[已脱敏]"));
    }
}
