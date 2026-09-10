//! Git object traversal and first-parent file statistics.

use crate::metrics::{
    CommitChange, MetricAccumulator, compute_bus_factor, compute_couplings, compute_language_totals,
};
use crate::model::{
    AnalysisConfig, AnalysisPhase, AnalysisStatus, AuthorSummary, CommitSummary, DailyActivity,
    Freshness, OverviewReport, RefSummary,
};
use crate::worktree::{
    WorktreeScan, WorktreeScanOptions, language_lines, scan_tree, scan_worktree,
};
use anyhow::{Context as _, Result};
use bstr::{BStr, ByteSlice};
use chrono::{DateTime, Utc};
use gix::diff::tree_with_rewrites::Change as TreeChange;
use gix::prelude::TreeDiffChangeExt;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

/// Options affecting Git history traversal.
#[derive(Debug, Clone)]
pub struct HistoryOptions {
    /// Whether remote refs are included.  RepoScope defaults to `true` to
    /// preserve the old `--all` behavior.
    pub include_remote_refs: bool,
    /// Optional upper bound used by fixture tests and diagnostic callers.
    pub max_commits: Option<usize>,
}

impl Default for HistoryOptions {
    fn default() -> Self {
        Self {
            include_remote_refs: true,
            max_commits: None,
        }
    }
}

/// Fingerprints captured before and after a scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryFingerprint {
    /// Hash of visible ref names and target ids.
    pub refs: String,
    /// Hash of worktree paths and metadata, or `None` for a bare repository.
    pub worktree: Option<String>,
}

impl RepositoryFingerprint {
    /// Capture a lightweight refs/worktree fingerprint for an existing local
    /// repository.  This reads Git metadata and filesystem metadata only; it
    /// never contacts a remote or starts a subprocess.
    pub fn for_repository(repository: impl AsRef<Path>) -> Result<Self> {
        let repo = gix::discover(repository.as_ref())
            .with_context(|| format!("发现 Git 仓库失败: {}", repository.as_ref().display()))?;
        repository_fingerprint(&repo)
    }
}

/// A read-only scanner for one repository path.
pub struct HistoryScanner {
    repository: PathBuf,
    config: AnalysisConfig,
    options: HistoryOptions,
}

impl HistoryScanner {
    /// Create a scanner with the default `--all` history semantics.
    #[must_use]
    pub fn new(repository: impl Into<PathBuf>, config: AnalysisConfig) -> Self {
        Self {
            repository: repository.into(),
            config,
            options: HistoryOptions::default(),
        }
    }

    /// Set explicit history options.
    #[must_use]
    pub fn with_options(mut self, options: HistoryOptions) -> Self {
        self.options = options;
        self
    }

    /// Scan Git history and the current worktree without spawning `git`.
    pub fn scan<F>(
        &self,
        cancellation: &crate::CancellationToken,
        mut progress: F,
    ) -> Result<crate::AnalysisResult>
    where
        F: FnMut(AnalysisStatus),
    {
        let repo = gix::discover(&self.repository)
            .with_context(|| format!("发现 Git 仓库失败: {}", self.repository.display()))?;
        let start_fingerprint = repository_fingerprint(&repo)?;
        let (worktree_root, scope_prefix) = scan_scope(&self.repository, &repo)?;
        let refs = visible_refs(&repo, self.options.include_remote_refs)?;
        let tips: Vec<_> = refs.iter().filter_map(|item| item.target.clone()).collect();
        let commit_ids = collect_commit_ids(
            &repo,
            &tips,
            self.options.max_commits,
            cancellation,
            &mut progress,
        )?;
        let mut commits_with_changes = Vec::with_capacity(commit_ids.len());
        let mut accumulator = MetricAccumulator::new();
        let mut daily: BTreeMap<String, (u64, u64, u64, BTreeSet<String>)> = BTreeMap::new();

        for (index, commit_id) in commit_ids.iter().enumerate() {
            if cancellation.is_cancelled() {
                anyhow::bail!("分析已取消");
            }
            let commit = repo
                .find_commit(*commit_id)
                .with_context(|| format!("读取提交对象失败: {}", commit_id.to_hex()))?;
            let author = commit.author().context("读取提交作者失败")?;
            let author_name = author.name.to_str_lossy().to_string();
            let author_email = author.email.to_str_lossy().to_string();
            let authored_at = author
                .time()
                .context("读取提交时间失败")?
                .seconds
                .saturating_mul(1_000);
            let subject = commit_subject(commit.message_raw()?);
            let parent = commit.parent_ids().next().map(|id| id.detach());
            let changes = commit_changes(&repo, parent, *commit_id, scope_prefix.as_deref())?;
            let summary = CommitSummary {
                oid: commit_id.to_hex().to_string(),
                short_oid: commit_id.to_hex_with_len(7).to_string(),
                subject: redact_text(&subject),
                author_name: redact_text(&author_name),
                author_email: redact_email(&author_email),
                authored_at,
                parent_count: commit.parent_ids().count() as u32,
                additions: changes.iter().map(|change| change.additions).sum(),
                deletions: changes.iter().map(|change| change.deletions).sum(),
                files_changed: changes.len() as u64,
                category: crate::classify_commit(&subject).as_str().to_owned(),
            };
            let day = utc_day(authored_at);
            let day_totals = daily.entry(day).or_default();
            day_totals.0 = day_totals.0.saturating_add(1);
            day_totals.1 = day_totals.1.saturating_add(summary.additions);
            day_totals.2 = day_totals.2.saturating_add(summary.deletions);
            day_totals
                .3
                .insert(format!("{author_name}\0{author_email}"));
            commits_with_changes.push((summary, changes));
            progress(AnalysisStatus::running(
                "",
                AnalysisPhase::History,
                (index + 1) as u64,
                commit_ids.len() as u64,
            ));
        }

        // Rename detection is local to each tree diff.  Resolve the resulting
        // aliases once, oldest-to-newest, so a chain such as A -> B -> C is
        // represented by the final path C in every metric and coupling query.
        normalize_rename_chains(&mut commits_with_changes);
        for (summary, changes) in &commits_with_changes {
            accumulator.ingest_commit(
                &summary.oid,
                &summary.author_name,
                &summary.author_email,
                summary.authored_at,
                changes,
            );
        }

        progress(AnalysisStatus::running("", AnalysisPhase::Worktree, 0, 1));
        let worktree_options = WorktreeScanOptions {
            max_file_bytes: self.config.max_file_bytes,
        };
        let mut worktree = if let Some(path) = worktree_root.as_deref() {
            scan_worktree(path, worktree_options)?
        } else {
            // A bare repository has no filesystem worktree.  Analyze the
            // current HEAD tree in-place so its code-size and language totals
            // remain useful while write-oriented GitButler actions stay hidden
            // by the UI.  An unborn/empty bare repository has no tree object
            // at all; treat that case as an empty snapshot instead of trying
            // to materialize the well-known empty-tree id from an object store
            // that cannot contain it yet.
            match repo.head_tree_id() {
                Ok(tree_id) => scan_tree(&repo, tree_id.detach(), worktree_options)?,
                Err(error) => {
                    tracing::debug!(?error, "裸仓库没有可分析的 HEAD 树，使用空工作树");
                    WorktreeScan::default()
                }
            }
        };
        let tracked_paths = if repo.workdir().is_some() {
            crate::tracked_paths(&self.repository)
        } else {
            BTreeSet::new()
        };
        for file in &mut worktree.files {
            // A committed tree is, by definition, tracked.  Files discovered
            // from a real worktree are annotated from the index so ignored and
            // untracked source files remain visible without being confused
            // with committed code.
            file.tracked = repo.workdir().is_none()
                || tracked_path_in_scope(&tracked_paths, &file.path, scope_prefix.as_deref());
        }
        worktree.summary.untracked_files =
            worktree.files.iter().filter(|file| !file.tracked).count() as u64;
        let mut current_lines = HashMap::new();
        let mut language_by_path = HashMap::new();
        for file in &worktree.files {
            current_lines.insert(file.path.clone(), file.lines);
            language_by_path.insert(file.path.clone(), file.language.clone());
            accumulator.set_current_file(&file.path, file.language.as_deref(), file.lines);
        }
        progress(AnalysisStatus::running("", AnalysisPhase::Metrics, 1, 1));

        let mut files = accumulator.files(&current_lines);
        for file in &mut files {
            file.language = language_by_path.get(&file.path).cloned().flatten();
            if let Some(worktree_file) = worktree.files.iter().find(|item| item.path == file.path) {
                file.raw_path_hex.clone_from(&worktree_file.raw_path_hex);
                file.tracked = worktree_file.tracked;
            }
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        progress(AnalysisStatus::running(
            "",
            AnalysisPhase::Ownership,
            0,
            worktree.files.len() as u64,
        ));
        let ownership = crate::analyze_ownership_in_scope(
            &self.repository,
            &worktree.files,
            self.config.ownership_max_files,
            self.config.workers,
            cancellation,
            scope_prefix.as_deref(),
            |processed, total| {
                progress(AnalysisStatus::running(
                    "",
                    AnalysisPhase::Ownership,
                    processed,
                    total,
                ));
            },
        )?;
        let mut authors = accumulator.authors();
        let owned_lines = ownership
            .owner_lines
            .iter()
            .cloned()
            .collect::<HashMap<_, _>>();
        for author in &mut authors {
            author.owned_lines = owned_lines.get(&author.identity).copied().unwrap_or(0);
        }
        let languages = compute_language_totals(language_lines(&worktree));
        let activity = daily
            .into_iter()
            .map(
                |(date, (commits, additions, deletions, active_authors))| DailyActivity {
                    date,
                    commits,
                    additions,
                    deletions,
                    active_authors: active_authors.len() as u64,
                },
            )
            .collect::<Vec<_>>();
        let commits = commits_with_changes
            .iter()
            .map(|(summary, _)| summary.clone())
            .collect::<Vec<_>>();
        let overview = overview(
            &commits_with_changes,
            &files,
            &authors,
            &refs,
            &worktree,
            repo.workdir().is_none(),
        );
        let couplings = compute_couplings(
            commits_with_changes.iter().map(|(_, changes)| {
                changes
                    .iter()
                    .map(|change| change.path.clone())
                    .collect::<Vec<_>>()
            }),
            self.config.coupling_max_files,
            self.config.coupling_top_pairs,
        );
        let commit_counts: Vec<_> = authors
            .iter()
            .map(|author| (author.identity.clone(), author.commit_count))
            .collect();
        let owned_lines: Vec<_> = authors
            .iter()
            .map(|author| (author.identity.clone(), author.owned_lines))
            .collect();
        let bus_factor = compute_bus_factor(&owned_lines, &commit_counts);
        progress(AnalysisStatus::running("", AnalysisPhase::Evidence, 0, 1));
        let evidence = crate::analyze_evidence(
            worktree_root.as_deref().unwrap_or(&self.repository),
            repo.workdir().is_none(),
            &commits,
            &refs,
            cancellation,
            |processed, total| {
                progress(AnalysisStatus::running(
                    "",
                    AnalysisPhase::Evidence,
                    processed,
                    total,
                ));
            },
        );
        let (evidence, partial) = match evidence {
            Ok(reports) => (reports, false),
            Err(error) if cancellation.is_cancelled() => return Err(error),
            Err(error) => {
                let reason = format!("可选审计报告失败：{}", redact_text(&error.to_string()));
                (
                    crate::EvidenceReports::unavailable(
                        commits.as_slice(),
                        refs.as_slice(),
                        reason,
                    ),
                    true,
                )
            }
        };
        let end_fingerprint = repository_fingerprint(&repo)?;
        let freshness = compare_fingerprints(&start_fingerprint, &end_fingerprint);
        let commit_files = commits_with_changes
            .iter()
            .map(|(summary, changes)| (summary.oid.clone(), changes.clone()))
            .collect();
        Ok(crate::AnalysisResult {
            run_id: String::new(),
            overview,
            commits,
            commit_files,
            files,
            authors,
            languages,
            activity,
            refs,
            couplings,
            bus_factor,
            worktree: worktree.summary,
            ownership: ownership.rows,
            directory_ownership: ownership.directories,
            code_age: ownership.code_age,
            file_ages: ownership.file_ages,
            ownership_coverage: ownership.coverage,
            freshness,
            start_fingerprint,
            end_fingerprint,
            delivery: evidence.delivery,
            footprints: evidence.footprints,
            regions: evidence.regions,
            dependencies: evidence.dependencies,
            partial,
        })
    }
}

fn collect_commit_ids<F>(
    repo: &gix::Repository,
    tips: &[String],
    max_commits: Option<usize>,
    cancellation: &crate::CancellationToken,
    progress: &mut F,
) -> Result<Vec<gix::ObjectId>>
where
    F: FnMut(AnalysisStatus),
{
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    let total_tips = tips.len() as u64;
    for (tip_index, tip) in tips.iter().enumerate() {
        let id = parse_object_id(repo, tip)?;
        let walk = repo.rev_walk(Some(id)).all()?;
        for info in walk {
            if cancellation.is_cancelled() {
                anyhow::bail!("分析已取消");
            }
            let info = info?;
            if seen.insert(info.id) {
                ids.push(info.id);
                if max_commits.is_some_and(|limit| ids.len() >= limit) {
                    break;
                }
            }
        }
        progress(AnalysisStatus::running(
            "",
            AnalysisPhase::History,
            (tip_index + 1) as u64,
            total_tips,
        ));
        if max_commits.is_some_and(|limit| ids.len() >= limit) {
            break;
        }
    }
    ids.sort_by(|left, right| {
        let left_time = repo
            .find_commit(*left)
            .ok()
            .and_then(|commit| commit.author().ok()?.time().ok().map(|time| time.seconds));
        let right_time = repo
            .find_commit(*right)
            .ok()
            .and_then(|commit| commit.author().ok()?.time().ok().map(|time| time.seconds));
        right_time.cmp(&left_time).then_with(|| left.cmp(right))
    });
    Ok(ids)
}

fn parse_object_id(repo: &gix::Repository, hex: &str) -> Result<gix::ObjectId> {
    let id = gix::ObjectId::from_hex(hex.as_bytes())
        .with_context(|| format!("解析 ref 目标失败: {hex}"))?;
    if !repo.has_object(id) {
        anyhow::bail!("ref 目标对象不存在: {hex}");
    }
    Ok(id)
}

fn commit_changes(
    repo: &gix::Repository,
    parent: Option<gix::ObjectId>,
    commit_id: gix::ObjectId,
    scope_prefix: Option<&str>,
) -> Result<Vec<CommitChange>> {
    let commit = repo.find_commit(commit_id)?;
    let tree = commit.tree()?;
    let parent_tree = match parent {
        Some(id) => {
            let parent_commit = repo
                .find_commit(id)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            Some(
                parent_commit
                    .tree()
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?,
            )
        }
        None => None,
    };
    let changes = repo.diff_tree_to_tree(parent_tree.as_ref(), &tree, None)?;
    let mut resource_cache = repo.diff_resource_cache_for_tree_diff()?;
    let mut result = Vec::new();
    for change in changes {
        if change.entry_mode().is_tree() {
            continue;
        }
        let Some(path) = scoped_git_path(change.location(), scope_prefix) else {
            continue;
        };
        let (additions, deletions) = change
            .attach(repo, repo)
            .diff(&mut resource_cache)
            .ok()
            .and_then(|mut diff| diff.line_counts().ok())
            .flatten()
            .map_or((0, 0), |counts| {
                (u64::from(counts.insertions), u64::from(counts.removals))
            });
        let previous_path = match &change {
            TreeChange::Rewrite {
                source_location, ..
            } => scoped_git_path(source_location.as_bstr(), scope_prefix),
            _ => None,
        };
        result.push(CommitChange {
            path,
            previous_path,
            additions,
            deletions,
        });
        resource_cache.clear_resource_cache_keep_allocation();
    }
    result.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(result)
}

/// Resolve a requested path to a worktree scope and repository-relative
/// prefix.  Git discovery returns the repository root even when the user
/// selected a nested directory, so this boundary prevents sibling files from
/// entering the analysis.
fn scan_scope(
    requested: &Path,
    repo: &gix::Repository,
) -> Result<(Option<PathBuf>, Option<String>)> {
    let Some(worktree) = repo.workdir() else {
        return Ok((None, None));
    };
    let root = worktree
        .canonicalize()
        .unwrap_or_else(|_| worktree.to_owned());
    let requested = requested
        .canonicalize()
        .unwrap_or_else(|_| requested.to_owned());
    let requested_directory = if requested.is_dir() {
        requested
    } else {
        requested
            .parent()
            .map(Path::to_owned)
            .unwrap_or_else(|| root.clone())
    };
    if !requested_directory.starts_with(&root) {
        // A .git path or another repository handle may not be below the
        // worktree.  Falling back to the root keeps a valid project usable.
        return Ok((Some(root), None));
    }
    let relative = requested_directory
        .strip_prefix(&root)
        .map_or_else(
            |_| String::new(),
            |path| {
                path.to_string_lossy()
                    .replace('\\', "/")
                    .trim_matches('/')
                    .to_owned()
            },
        );
    if relative.is_empty() {
        Ok((Some(root), None))
    } else {
        Ok((Some(requested_directory), Some(relative)))
    }
}

fn scoped_git_path(path: &BStr, scope_prefix: Option<&str>) -> Option<String> {
    let path = path.to_str_lossy().replace('\\', "/");
    let Some(prefix) = scope_prefix
        .map(|value| value.trim_matches('/'))
        .filter(|value| !value.is_empty())
    else {
        return Some(path);
    };
    let prefix = prefix.trim_matches('/');
    path.strip_prefix(prefix)
        .and_then(|suffix| suffix.strip_prefix('/'))
        .map(ToOwned::to_owned)
}

fn tracked_path_in_scope(
    tracked_paths: &BTreeSet<String>,
    path: &str,
    scope_prefix: Option<&str>,
) -> bool {
    let Some(prefix) = scope_prefix
        .map(|value| value.trim_matches('/'))
        .filter(|value| !value.is_empty())
    else {
        return tracked_paths.contains(path);
    };
    tracked_paths.contains(&format!("{}/{}", prefix.trim_matches('/'), path))
}

fn normalize_rename_chains(commits: &mut [(CommitSummary, Vec<CommitChange>)]) {
    let mut aliases: HashMap<String, String> = HashMap::new();
    // The scanner keeps commits newest-first for the UI.  Alias updates must
    // happen in the opposite direction so every intermediate name resolves to
    // the final path that exists in the current tree.
    for (_, changes) in commits.iter().rev() {
        for change in changes {
            if let Some(previous) = &change.previous_path {
                let source = resolve_alias(previous, &aliases);
                aliases.insert(previous.clone(), change.path.clone());
                aliases.insert(source, change.path.clone());
            }
        }
    }
    for (_, changes) in commits {
        for change in changes {
            change.path = resolve_alias(&change.path, &aliases);
            change.previous_path = change
                .previous_path
                .as_ref()
                .map(|path| resolve_alias(path, &aliases));
        }
    }
}

fn resolve_alias(path: &str, aliases: &HashMap<String, String>) -> String {
    let mut current = path.to_owned();
    let mut seen = HashSet::new();
    while let Some(next) = aliases.get(&current) {
        if !seen.insert(current.clone()) {
            break;
        }
        current = next.clone();
    }
    current
}

fn visible_refs(repo: &gix::Repository, include_remote_refs: bool) -> Result<Vec<RefSummary>> {
    let mut refs = Vec::new();
    for reference in repo.references()?.all()? {
        let reference = reference.map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let name = reference.name().as_bstr().to_owned();
        let excluded = is_internal_ref(name.as_ref());
        let remote = name.starts_with(b"refs/remotes/");
        let target = if !excluded && (include_remote_refs || !remote) {
            reference
                .into_fully_peeled_id()
                .ok()
                .map(|id| id.detach())
                // A ref may legally point at a tree or blob (for example a
                // manually-created testing ref).  Only commit tips are valid
                // history traversal roots; keep the ref visible but do not
                // let one malformed tip abort the entire analysis.
                .filter(|id| repo.find_commit(*id).is_ok())
                .map(|id| id.to_hex().to_string())
        } else {
            None
        };
        refs.push(RefSummary {
            name: name.to_str_lossy().to_string(),
            raw_name_hex: (!name.is_utf8()).then(|| hex::encode(name)),
            target,
            excluded,
        });
    }
    refs.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(refs)
}

fn is_internal_ref(name: &BStr) -> bool {
    name.starts_with(b"refs/gitbutler/")
        || name.starts_with(b"refs/heads/gitbutler/")
        || name.starts_with(b"refs/remotes/gitbutler/")
        // GitButler stores branch-local stashes in a Git ref namespace.  The
        // namespace is visible to a raw `refs --all` walk but is an internal
        // implementation detail and must not inflate contribution metrics.
        || name.starts_with(b"refs/namespaces/gitbutler-stashes/")
        || name.starts_with(b"refs/namespaces/gitbutler/")
}

fn repository_fingerprint(repo: &gix::Repository) -> Result<RepositoryFingerprint> {
    let references = repo.references()?;
    let refs = references.all()?;
    let mut ref_hash = Sha256::new();
    for reference in refs {
        let reference = reference.map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let name = reference.name().as_bstr().to_owned();
        if is_internal_ref(name.as_ref()) {
            continue;
        }
        ref_hash.update(name);
        if let Ok(id) = reference.into_fully_peeled_id() {
            ref_hash.update(id.detach().as_bytes());
        }
        ref_hash.update([0]);
    }
    let worktree = repo.workdir().map(worktree_fingerprint).transpose()?;
    Ok(RepositoryFingerprint {
        refs: hex::encode(ref_hash.finalize()),
        worktree,
    })
}

fn worktree_fingerprint(root: &Path) -> Result<String> {
    let mut entries = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory)
            .with_context(|| format!("读取工作树失败: {}", directory.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                entries.push((relative_path(root, &path), 0, 0));
            } else if file_type.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name == ".git" || name == ".gitbutler" {
                    continue;
                }
                stack.push(path);
            } else if file_type.is_file() {
                let metadata = entry.metadata()?;
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |duration| duration.as_nanos());
                entries.push((relative_path(root, &path), metadata.len(), modified));
            }
        }
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hash = Sha256::new();
    for (path, size, modified) in entries {
        hash.update(path.as_bytes());
        hash.update(size.to_le_bytes());
        hash.update(modified.to_le_bytes());
        hash.update([0]);
    }
    Ok(hex::encode(hash.finalize()))
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Compare two scan fingerprints and classify which part became stale.
#[must_use]
pub fn compare_fingerprints(
    start: &RepositoryFingerprint,
    end: &RepositoryFingerprint,
) -> Freshness {
    match (start.refs == end.refs, start.worktree == end.worktree) {
        (true, true) => Freshness::Fresh,
        (false, true) => Freshness::HistoryStale,
        (true, false) => Freshness::WorktreeStale,
        (false, false) => Freshness::Interrupted,
    }
}

fn commit_subject(message: &BStr) -> String {
    message
        .split(|byte| *byte == b'\n' || *byte == b'\r')
        .next()
        .unwrap_or_default()
        .to_str_lossy()
        .trim()
        .to_owned()
}

fn utc_day(timestamp_ms: i64) -> String {
    DateTime::<Utc>::from_timestamp(timestamp_ms.div_euclid(1_000), 0).map_or_else(
        || "1970-01-01".to_owned(),
        |date| date.format("%Y-%m-%d").to_string(),
    )
}

fn redact_text(text: &str) -> String {
    let value = text.replace(['\r', '\n', '\0'], " ");
    static URL_USERINFO: OnceLock<Regex> = OnceLock::new();
    static BEARER: OnceLock<Regex> = OnceLock::new();
    static SECRET: OnceLock<Regex> = OnceLock::new();
    let value = URL_USERINFO
        .get_or_init(|| {
            Regex::new(r#"(?i)(\b[a-z][a-z0-9+.-]*://)[^/\s:@]+(?::[^/\s@]*)?@"#)
                .expect("valid history URL credential regex")
        })
        .replace_all(&value, "$1[已脱敏]@");
    let value = BEARER
        .get_or_init(|| {
            Regex::new(r#"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+"#)
                .expect("valid history bearer credential regex")
        })
        .replace_all(&value, "Bearer [已脱敏]");
    SECRET
        .get_or_init(|| {
            Regex::new(
                r#"(?i)["']?(user(?:name)?|login|password|passwd|pwd|token|secret|api[_-]?key|access[_-]?token|client[_-]?secret|authorization|private[_-]?key)["']?\s*([:=])\s*(?:"[^"]*"|'[^']*'|[^,;\s}\]]+)"#,
            )
            .expect("valid history redaction regex")
        })
        .replace_all(&value, "$1$2[已脱敏]")
        .into_owned()
}

fn redact_email(email: &str) -> String {
    let Some((local, domain)) = email.split_once('@') else {
        return "[已脱敏]".to_owned();
    };
    let visible = local.chars().next().unwrap_or('*');
    format!("{visible}***@{domain}")
}

fn overview(
    commits: &[(CommitSummary, Vec<CommitChange>)],
    files: &[crate::model::FileSummary],
    authors: &[AuthorSummary],
    refs: &[RefSummary],
    worktree: &WorktreeScan,
    bare: bool,
) -> OverviewReport {
    let additions = commits.iter().map(|(commit, _)| commit.additions).sum();
    let deletions = commits.iter().map(|(commit, _)| commit.deletions).sum();
    let first_commit_at = commits.iter().map(|(commit, _)| commit.authored_at).min();
    let last_commit_at = commits.iter().map(|(commit, _)| commit.authored_at).max();
    OverviewReport {
        commit_count: commits.len() as u64,
        file_count: files.len() as u64,
        author_count: authors.len() as u64,
        branch_count: refs
            .iter()
            .filter(|reference| !reference.excluded && reference.name.starts_with("refs/heads/"))
            .count() as u64,
        tag_count: refs
            .iter()
            .filter(|reference| !reference.excluded && reference.name.starts_with("refs/tags/"))
            .count() as u64,
        untracked_files: worktree.summary.untracked_files,
        lines: worktree.summary.lines,
        additions,
        deletions,
        churn: additions.saturating_add(deletions),
        net_growth: additions as i64 - deletions as i64,
        first_commit_at,
        last_commit_at,
        bare,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_refs_are_hidden() {
        assert!(is_internal_ref(BStr::new(b"refs/gitbutler/workspace")));
        assert!(is_internal_ref(BStr::new(
            b"refs/namespaces/gitbutler-stashes/refs/heads/gitbutler/workspace"
        )));
        assert!(!is_internal_ref(BStr::new(b"refs/heads/main")));
    }

    #[test]
    fn fingerprints_report_worktree_changes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::create_dir(dir.path().join(".git"))?;
        // This only exercises the deterministic helper's path handling; a real
        // gix repository is covered by the integration fixtures.
        let first = worktree_fingerprint(dir.path())?;
        std::fs::write(dir.path().join("a.rs"), "fn main() {}")?;
        let second = worktree_fingerprint(dir.path())?;
        assert_ne!(
            first, second,
            "adding a source file changes the fingerprint"
        );
        Ok(())
    }

    #[test]
    fn empty_bare_repository_is_an_empty_analysis() -> Result<()> {
        let dir = tempfile::tempdir()?;
        gix::init_bare(dir.path())?;
        let scanner = HistoryScanner::new(dir.path(), crate::model::AnalysisConfig::default());
        let result = scanner.scan(&crate::CancellationToken::new(), |_| {})?;
        assert!(result.overview.bare);
        assert_eq!(result.overview.commit_count, 0);
        assert_eq!(result.overview.file_count, 0);
        assert!(result.files.is_empty());
        Ok(())
    }

    #[test]
    fn redaction_handles_quoted_and_unquoted_credentials() {
        let value = redact_text(r#"token: "secret-value", password='hunter2'"#);
        assert!(!value.contains("secret-value"));
        assert!(!value.contains("hunter2"));
        assert_eq!(value.matches("[已脱敏]").count(), 2);
    }

    #[test]
    fn nested_repository_input_is_scoped_without_sibling_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let repo = gix::init(dir.path())?;
        let nested = dir.path().join("src");
        std::fs::create_dir(&nested)?;
        let (scope, prefix) = scan_scope(&nested, &repo)?;
        assert_eq!(prefix.as_deref(), Some("src"));
        assert_eq!(scope.as_deref(), Some(nested.as_path()));
        assert_eq!(
            scoped_git_path(BStr::new(b"src/main.rs"), prefix.as_deref()).as_deref(),
            Some("main.rs")
        );
        assert!(scoped_git_path(BStr::new(b"tests/main.rs"), prefix.as_deref()).is_none());
        Ok(())
    }
}
