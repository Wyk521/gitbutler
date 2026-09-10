//! Offline audit evidence scanners.
//!
//! The scanners in this module intentionally operate on bytes already present
//! on disk.  They do not resolve domains, contact package registries, execute
//! build tools, or run Git hooks.  Every value is redacted before it becomes a
//! crate::model::EvidenceItem.

use crate::CancellationToken;
use crate::model::{CommitSummary, EvidenceItem, RefSummary, ReportSnapshot};
use anyhow::{Context as _, Result};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;
use url::Url;

const MAX_SCAN_BYTES: u64 = 150_000_000;
const MAX_FILE_BYTES: u64 = 3_000_000;
const MAX_EVIDENCE: usize = 4_000;
const MAX_MANIFESTS: usize = 1_000;

const EXCLUDED_DIRECTORIES: &[&str] = &[
    ".git",
    ".gitbutler",
    "node_modules",
    "vendor",
    "dist",
    "build",
    "target",
    "out",
    "bin",
    "obj",
    ".venv",
    "venv",
    "coverage",
    ".next",
    ".nuxt",
    ".cache",
    ".turbo",
    "__pycache__",
];

const MANIFEST_NAMES: &[&str] = &[
    "package.json",
    "package-lock.json",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "libs.versions.toml",
    "pyproject.toml",
    "go.mod",
    "cargo.toml",
    "composer.json",
    "gemfile",
    "packages.config",
    "package.swift",
];

/// Results from all optional, read-only audit scanners.
#[derive(Debug, Clone, Default)]
pub struct EvidenceReports {
    /// Delivery candidate report.
    pub delivery: ReportSnapshot,
    /// External system and sensitive-footprint report.
    pub footprints: ReportSnapshot,
    /// Regional string clue report.
    pub regions: ReportSnapshot,
    /// Dependency and plugin report.
    pub dependencies: ReportSnapshot,
}

impl EvidenceReports {
    /// Build a result that preserves delivery metadata while recording a
    /// failure for the optional filesystem evidence stages.  Core Git metrics
    /// can therefore be activated as a `partial` run without hiding the last
    /// successful batch.
    pub fn unavailable(commits: &[CommitSummary], refs: &[RefSummary], reason: String) -> Self {
        Self {
            delivery: delivery_report(commits, refs),
            footprints: unavailable_report("footprints", &reason),
            regions: unavailable_report("regions", &reason),
            dependencies: unavailable_report("dependencies", &reason),
        }
    }
}

/// Scan the current worktree for the four RepoScope audit reports.
pub fn analyze_evidence<F>(
    repository: impl AsRef<Path>,
    bare: bool,
    commits: &[CommitSummary],
    refs: &[RefSummary],
    cancellation: &CancellationToken,
    mut progress: F,
) -> Result<EvidenceReports>
where
    F: FnMut(u64, u64),
{
    if bare {
        return Ok(EvidenceReports {
            delivery: delivery_report(commits, refs),
            footprints: unavailable_report(
                "footprints",
                "裸仓库没有工作目录，无法扫描外部系统线索。",
            ),
            regions: unavailable_report("regions", "裸仓库没有工作目录，无法扫描地域字符串。"),
            dependencies: unavailable_report(
                "dependencies",
                "裸仓库没有工作目录，无法读取依赖清单。",
            ),
        });
    }
    let files = collect_text_files(repository.as_ref(), cancellation)?;
    let total = files.len() as u64;
    let mut footprint_items = Vec::new();
    let mut region_items = Vec::new();
    let mut dependency_items = Vec::new();
    let mut scanned_files = 0_u64;
    let mut scanned_bytes = 0_u64;
    let mut truncated = false;

    for (index, (path, content, bytes)) in files.iter().enumerate() {
        if cancellation.is_cancelled() {
            anyhow::bail!("分析已取消");
        }
        scanned_files = scanned_files.saturating_add(1);
        scanned_bytes = scanned_bytes.saturating_add(*bytes);
        if scanned_bytes > MAX_SCAN_BYTES {
            truncated = true;
            break;
        }
        parse_dependencies(path, content, &mut dependency_items);
        scan_footprints(path, content, &mut footprint_items);
        scan_regions(path, content, &mut region_items);
        if footprint_items.len() >= MAX_EVIDENCE {
            footprint_items.truncate(MAX_EVIDENCE);
            truncated = true;
        }
        if region_items.len() >= MAX_EVIDENCE {
            region_items.truncate(MAX_EVIDENCE);
            truncated = true;
        }
        if dependency_items.len() >= MAX_EVIDENCE {
            dependency_items.truncate(MAX_EVIDENCE);
            truncated = true;
        }
        progress((index + 1) as u64, total);
    }
    deduplicate_items(&mut footprint_items);
    deduplicate_items(&mut region_items);
    deduplicate_items(&mut dependency_items);

    let mut reports = EvidenceReports {
        delivery: delivery_report(commits, refs),
        footprints: report(
            "footprints",
            footprint_items,
            scanned_files,
            truncated,
            "外部系统地址、网络端点和客户/租户标识仅作为待核验线索，不等同于项目归属证明。",
        ),
        regions: report(
            "regions",
            region_items,
            scanned_files,
            truncated,
            "地域名称和拼音是源码字符串证据，可能来自测试数据、字典或通用业务，不能据此认定项目归属。",
        ),
        dependencies: report(
            "dependencies",
            dependency_items,
            scanned_files,
            truncated,
            "依赖版本来自仓库清单；未固定版本需要结合锁文件和实际构建环境继续核验。",
        ),
    };
    reports.footprints.available = true;
    reports.regions.available = true;
    reports.dependencies.available = true;
    Ok(reports)
}

fn collect_text_files(
    root: &Path,
    cancellation: &CancellationToken,
) -> Result<Vec<(String, String, u64)>> {
    let root = root
        .canonicalize()
        .with_context(|| format!("解析证据扫描根目录失败: {}", root.display()))?;
    let mut stack = vec![root.clone()];
    let mut files = Vec::new();
    let mut scanned_bytes = 0_u64;
    while let Some(directory) = stack.pop() {
        if cancellation.is_cancelled() {
            anyhow::bail!("分析已取消");
        }
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                tracing::debug!(path = %directory.display(), ?error, "跳过不可读的证据目录");
                continue;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(kind) => kind,
                Err(_) => continue,
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if !is_excluded_directory(&entry.file_name().to_string_lossy()) {
                    stack.push(path);
                }
                continue;
            }
            if !file_type.is_file() || files.len() >= MAX_MANIFESTS.saturating_mul(20) {
                continue;
            }
            let metadata = match entry.metadata() {
                Ok(metadata) if metadata.len() <= MAX_FILE_BYTES => metadata,
                _ => continue,
            };
            // A file can grow after the metadata check.  Read one byte past
            // the limit so a race cannot make the evidence scanner retain an
            // unbounded payload or exceed its total scan budget.
            let mut file = match fs::File::open(&path) {
                Ok(file) => file,
                Err(_) => continue,
            };
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            if file
                .by_ref()
                .take(MAX_FILE_BYTES.saturating_add(1))
                .read_to_end(&mut bytes)
                .is_err()
                || bytes.len() as u64 > MAX_FILE_BYTES
            {
                continue;
            }
            if bytes.get(..8_192).is_some_and(|head| head.contains(&0)) {
                continue;
            }
            scanned_bytes = scanned_bytes.saturating_add(bytes.len() as u64);
            if scanned_bytes > MAX_SCAN_BYTES {
                break;
            }
            let relative = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            files.push((
                relative,
                String::from_utf8_lossy(&bytes).into_owned(),
                bytes.len() as u64,
            ));
        }
        if scanned_bytes > MAX_SCAN_BYTES || files.len() >= MAX_MANIFESTS.saturating_mul(20) {
            break;
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn is_excluded_directory(name: &str) -> bool {
    EXCLUDED_DIRECTORIES
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(name))
}

fn is_manifest(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    MANIFEST_NAMES.iter().any(|item| *item == name)
        || name.ends_with(".csproj")
        || (name.starts_with("requirements") && name.ends_with(".txt"))
}

fn parse_dependencies(path: &str, content: &str, output: &mut Vec<EvidenceItem>) {
    if !is_manifest(path) {
        return;
    }
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    match name.as_str() {
        "package.json" | "package-lock.json" => parse_package_json(path, content, output),
        "cargo.toml" | "pyproject.toml" | "libs.versions.toml" => {
            parse_toml_manifest(path, content, output)
        }
        "go.mod" => parse_go_mod(path, content, output),
        "pom.xml" | "packages.config" => parse_xml_manifest(path, content, output),
        "build.gradle" | "build.gradle.kts" => parse_gradle(path, content, output),
        "composer.json" => parse_json_groups(path, content, output, "Composer"),
        "gemfile" => parse_gemfile(path, content, output),
        "package.swift" => parse_swift(path, content, output),
        _ if name.starts_with("requirements") => parse_requirements(path, content, output),
        _ if name.ends_with(".csproj") => parse_xml_manifest(path, content, output),
        _ => {}
    }
}

fn parse_package_json(path: &str, content: &str, output: &mut Vec<EvidenceItem>) {
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(content) else {
        return;
    };
    for (group, scope, kind) in [
        ("dependencies", "运行时", "依赖"),
        ("devDependencies", "开发", "依赖"),
        ("peerDependencies", "Peer", "依赖"),
        ("optionalDependencies", "可选", "依赖"),
        ("bundledDependencies", "内置", "依赖"),
    ] {
        let Some(value) = root.get(group) else {
            continue;
        };
        match value {
            Value::Object(values) => {
                for (name, version) in values {
                    output.push(dependency_item(
                        "npm",
                        name,
                        version.as_str().unwrap_or("未指定"),
                        scope,
                        kind,
                        path,
                        line_for(content, name),
                    ));
                }
            }
            Value::Array(values) => {
                for name in values.iter().filter_map(Value::as_str) {
                    output.push(dependency_item(
                        "npm",
                        name,
                        "内置",
                        scope,
                        kind,
                        path,
                        line_for(content, name),
                    ));
                }
            }
            _ => {}
        }
    }
}

fn parse_json_groups(path: &str, content: &str, output: &mut Vec<EvidenceItem>, ecosystem: &str) {
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(content) else {
        return;
    };
    for (group, scope) in [("require", "运行时"), ("require-dev", "开发")] {
        let Some(Value::Object(values)) = root.get(group) else {
            continue;
        };
        for (name, version) in values {
            if name == "php" {
                continue;
            }
            output.push(dependency_item(
                ecosystem,
                name,
                version.as_str().unwrap_or("未指定"),
                scope,
                "依赖",
                path,
                line_for(content, name),
            ));
        }
    }
}

fn parse_toml_manifest(path: &str, content: &str, output: &mut Vec<EvidenceItem>) {
    let Ok(value) = content.parse::<toml::Value>() else {
        return;
    };
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    if name == "cargo.toml" {
        for (group, scope) in [
            ("dependencies", "运行时"),
            ("dev-dependencies", "开发"),
            ("build-dependencies", "构建"),
        ] {
            parse_toml_table(&value, group, "Cargo", scope, "依赖", path, content, output);
        }
    } else if name == "pyproject.toml" {
        if let Some(array) = value
            .get("project")
            .and_then(|item| item.get("dependencies"))
            .and_then(toml::Value::as_array)
        {
            for item in array.iter().filter_map(toml::Value::as_str) {
                let (package, version) = split_requirement(item);
                output.push(dependency_item(
                    "Python",
                    package,
                    version,
                    "运行时",
                    "依赖",
                    path,
                    line_for(content, package),
                ));
            }
        }
        if let Some(table) = value
            .get("tool")
            .and_then(|item| item.get("poetry"))
            .and_then(|item| item.get("dependencies"))
            .and_then(toml::Value::as_table)
        {
            for (package, version) in table {
                if package.eq_ignore_ascii_case("python") {
                    continue;
                }
                output.push(dependency_item(
                    "Python",
                    package,
                    &toml_scalar(version),
                    "运行时",
                    "依赖",
                    path,
                    line_for(content, package),
                ));
            }
        }
    } else {
        parse_toml_table(
            &value,
            "libraries",
            "Gradle",
            "版本目录",
            "依赖",
            path,
            content,
            output,
        );
        parse_toml_table(
            &value, "plugins", "Gradle", "构建", "插件", path, content, output,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn parse_toml_table(
    root: &toml::Value,
    key: &str,
    ecosystem: &str,
    scope: &str,
    kind: &str,
    path: &str,
    content: &str,
    output: &mut Vec<EvidenceItem>,
) {
    let Some(table) = root.get(key).and_then(toml::Value::as_table) else {
        return;
    };
    for (name, value) in table {
        let display_name = if ecosystem == "Gradle" {
            value
                .get("module")
                .and_then(toml::Value::as_str)
                .unwrap_or(name)
        } else {
            name
        };
        let version = if ecosystem == "Gradle" {
            value
                .get("version")
                .map(toml_scalar)
                .or_else(|| value.get("version.ref").map(toml_scalar))
                .unwrap_or_else(|| "未指定".to_owned())
        } else {
            toml_scalar(value)
        };
        output.push(dependency_item(
            ecosystem,
            display_name,
            &version,
            scope,
            kind,
            path,
            line_for(content, name),
        ));
    }
}

fn parse_go_mod(path: &str, content: &str, output: &mut Vec<EvidenceItem>) {
    let re = Regex::new(r"(?m)^\s*([^\s()]+)\s+([^\s]+)(?:\s+//\s+indirect)?$")
        .expect("valid go.mod regex");
    let block = Regex::new(r"(?s)require\s*\((.*?)\)").expect("valid go block regex");
    for capture in block.captures_iter(content) {
        for line in capture[1].lines() {
            if let Some(values) = re.captures(line.trim()) {
                output.push(dependency_item(
                    "Go",
                    &values[1],
                    &values[2],
                    if line.contains("indirect") {
                        "间接"
                    } else {
                        "运行时"
                    },
                    "依赖",
                    path,
                    line_number(content, line),
                ));
            }
        }
    }
    let single = Regex::new(r"(?m)^\s*require\s+([^\s]+)\s+([^\s]+)")
        .expect("valid single go require regex");
    for values in single.captures_iter(content) {
        output.push(dependency_item(
            "Go",
            &values[1],
            &values[2],
            "运行时",
            "依赖",
            path,
            line_for(content, &values[1]),
        ));
    }
}

fn parse_xml_manifest(path: &str, content: &str, output: &mut Vec<EvidenceItem>) {
    let lower_path = path.to_ascii_lowercase();
    if lower_path.ends_with(".csproj") || lower_path.ends_with("packages.config") {
        let re = Regex::new(r#"(?i)<(?:PackageReference|package)\b[^>]*(?:Include|id)\s*=\s*['"]([^'"]+)['"][^>]*(?:Version|version)\s*=\s*['"]([^'"]+)['"]"#)
            .expect("valid dotnet dependency regex");
        for values in re.captures_iter(content) {
            output.push(dependency_item(
                "NuGet",
                &values[1],
                &values[2],
                "运行时",
                "依赖",
                path,
                line_for(content, &values[1]),
            ));
        }
        return;
    }
    let block = Regex::new(r"(?is)<(dependency|plugin)\b[^>]*>(.*?)</(?:dependency|plugin)>")
        .expect("valid maven dependency regex");
    let tag = Regex::new(r"(?is)<(groupId|artifactId|version|scope)>\s*([^<]+?)\s*</(?:groupId|artifactId|version|scope)>").expect("valid maven field regex");
    for block_match in block.captures_iter(content) {
        let mut fields = HashMap::new();
        for field in tag.captures_iter(&block_match[2]) {
            fields.insert(field[1].to_ascii_lowercase(), field[2].trim().to_owned());
        }
        let Some(artifact) = fields.get("artifactid") else {
            continue;
        };
        let name = fields
            .get("groupid")
            .map_or_else(|| artifact.clone(), |group| format!("{group}:{artifact}"));
        let version = fields
            .get("version")
            .cloned()
            .unwrap_or_else(|| "由父项目管理".to_owned());
        let kind = if block_match[1].eq_ignore_ascii_case("plugin") {
            "插件"
        } else {
            "依赖"
        };
        let scope = fields
            .get("scope")
            .map(String::as_str)
            .unwrap_or(if kind == "插件" {
                "构建"
            } else {
                "运行时"
            });
        output.push(dependency_item(
            "Maven",
            &name,
            &version,
            scope,
            kind,
            path,
            line_for(content, artifact),
        ));
    }
}

fn parse_gradle(path: &str, content: &str, output: &mut Vec<EvidenceItem>) {
    let dependency =
        Regex::new(r#"(?m)^\s*([A-Za-z][\w.-]*)\s*\(?\s*['"]([^:'"]+):([^:'"]+):([^'"]+)['"]"#)
            .expect("valid Gradle dependency regex");
    for values in dependency.captures_iter(content) {
        output.push(dependency_item(
            "Gradle",
            &format!("{}:{}", &values[2], &values[3]),
            &values[4],
            &values[1],
            "依赖",
            path,
            line_for(content, &values[2]),
        ));
    }
    let plugin =
        Regex::new(r#"(?i)id\s*\(?\s*['"]([^'"]+)['"]\s*\)?\s*version\s*['"]([^'"]+)['"]"#)
            .expect("valid Gradle plugin regex");
    for values in plugin.captures_iter(content) {
        output.push(dependency_item(
            "Gradle",
            &values[1],
            &values[2],
            "构建",
            "插件",
            path,
            line_for(content, &values[1]),
        ));
    }
}

fn parse_requirements(path: &str, content: &str, output: &mut Vec<EvidenceItem>) {
    for (index, line) in content.lines().enumerate() {
        let value = line.trim();
        if value.is_empty() || value.starts_with('#') || value.starts_with('-') {
            continue;
        }
        let (name, version) = split_requirement(value);
        if !name.is_empty() {
            output.push(dependency_item(
                "Python",
                name,
                version,
                "运行时",
                "依赖",
                path,
                (index + 1) as u64,
            ));
        }
    }
}

fn parse_gemfile(path: &str, content: &str, output: &mut Vec<EvidenceItem>) {
    let re =
        Regex::new(r#"(?m)^\s*gem\s+['"]([^'"]+)['"](?:\s*,\s*['"]([^'"]+)['"])? "#.trim_end())
            .expect("valid Gemfile regex");
    for values in re.captures_iter(content) {
        output.push(dependency_item(
            "RubyGems",
            &values[1],
            values.get(2).map_or("未指定", |value| value.as_str()),
            "运行时",
            "依赖",
            path,
            line_for(content, &values[1]),
        ));
    }
}

fn parse_swift(path: &str, content: &str, output: &mut Vec<EvidenceItem>) {
    let re = Regex::new(
        r#"\.package\s*\(\s*url:\s*['"]([^'"]+)['"]\s*,\s*(?:from|exact):\s*['"]([^'"]+)['"]"#,
    )
    .expect("valid SwiftPM regex");
    for values in re.captures_iter(content) {
        let name = values[1]
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(&values[1])
            .trim_end_matches(".git");
        output.push(dependency_item(
            "SwiftPM",
            name,
            &values[2],
            "运行时",
            "依赖",
            path,
            line_for(content, name),
        ));
    }
}

fn dependency_item(
    ecosystem: &str,
    name: &str,
    version: &str,
    scope: &str,
    kind: &str,
    path: &str,
    line: u64,
) -> EvidenceItem {
    // Dependency names and versions can legally be VCS URLs or inline
    // credentials (for example a private Git URL in a package manifest).
    // Redact both fields before constructing either the display value or the
    // structured metadata object.
    let name = clean_text(&redact_text(name));
    let version = clean_text(&redact_text(version));
    let unpinned = is_unpinned(&version);
    EvidenceItem {
        category: "dependency".to_owned(),
        kind: kind.to_owned(),
        value: format!("{ecosystem}: {name} {version}"),
        path: Some(path.to_owned()),
        line: Some(line),
        conclusion: if unpinned {
            "未固定版本，需结合锁文件和构建环境核验。".to_owned()
        } else {
            "清单中发现的依赖或插件。".to_owned()
        },
        metadata: Some(serde_json::json!({
            "ecosystem": ecosystem, "name": name, "version": version,
            "scope": scope, "kind": kind, "unpinned": unpinned,
        })),
    }
}

fn scan_footprints(path: &str, content: &str, output: &mut Vec<EvidenceItem>) {
    let url_re =
        Regex::new(r#"(?i)\b(?:https?|wss?|ftp|sftp)://[^\s'"]+"#).expect("valid URL regex");
    let connection_re = Regex::new(r#"(?i)\b(?:jdbc:(?:mysql|postgresql|sqlserver|oracle|mariadb|db2):|mongodb(?:\+srv)?://|redis(?:s)?://|amqps?://)[^\s'"]+"#).expect("valid connection regex");
    let ip_re = Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}(?::\d{2,5})?\b").expect("valid IP regex");
    let host_re = Regex::new(r#"(?i)\b(?:host|hostname|server|endpoint|base[_-]?url|api[_-]?url)\b\s*[:=]\s*['"]?([a-z0-9][a-z0-9.-]+(?::\d{2,5})?)"#).expect("valid host regex");
    let identity_re = Regex::new(r#"(?i)(?:client|customer|tenant|brand|project|system|客户|租户|项目|系统)(?:[_ .-]?(?:name|id|code|名称|编号))?\s*[:=]\s*['"]([^'"]{2,80})['"]"#).expect("valid identity regex");
    for (number, line) in content.lines().enumerate() {
        let mut matches: Vec<(String, bool)> = Vec::new();
        matches.extend(
            url_re
                .find_iter(line)
                .map(|item| (item.as_str().to_owned(), false)),
        );
        matches.extend(
            connection_re
                .find_iter(line)
                .map(|item| (item.as_str().to_owned(), false)),
        );
        matches.extend(
            ip_re
                .find_iter(line)
                .map(|item| (item.as_str().to_owned(), false)),
        );
        matches.extend(
            host_re
                .captures_iter(line)
                .map(|item| (item[1].to_owned(), false)),
        );
        matches.extend(
            identity_re
                .captures_iter(line)
                .map(|item| (item[1].trim().to_owned(), true)),
        );
        for (raw, identity) in matches {
            let raw = raw.trim_matches(|character: char| ".,;:)]}>".contains(character));
            if raw.is_empty() {
                continue;
            }
            let value = redact_value(raw);
            let host = if identity { String::new() } else { host(raw) };
            let category = footprint_category(raw, &host, identity);
            let (level, score) = footprint_risk(raw, &host, path, category);
            output.push(EvidenceItem {
                category: category.to_owned(),
                kind: level.to_owned(),
                value,
                path: Some(path.to_owned()),
                line: Some((number + 1) as u64),
                conclusion: "发现待核验的外部系统或身份线索。".to_owned(),
                metadata: Some(serde_json::json!({
                    "host": host, "risk": level, "riskScore": score,
                    "snippet": redact_text(line).chars().take(260).collect::<String>(),
                })),
            });
            if output.len() >= MAX_EVIDENCE {
                return;
            }
        }
    }
}

fn scan_regions(path: &str, content: &str, output: &mut Vec<EvidenceItem>) {
    for (province, full, alias, pinyin, level) in region_aliases() {
        if contains_alias(content, alias) || contains_alias(content, pinyin) {
            let hit = if contains_alias(content, alias) {
                alias
            } else {
                pinyin
            };
            output.push(EvidenceItem {
                category: "region".to_owned(),
                kind: level.to_owned(),
                value: format!("{province}（{full}）：{hit}"),
                path: Some(path.to_owned()),
                line: Some(line_for(content, hit)),
                conclusion: "地域字符串线索，仅供辅助判断。".to_owned(),
                metadata: Some(serde_json::json!({
                    "province": province, "provinceFull": full, "alias": hit, "pinyin": pinyin,
                })),
            });
            if output.len() >= MAX_EVIDENCE {
                return;
            }
        }
    }
}

fn region_aliases() -> Vec<(
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
)> {
    PROVINCES
        .iter()
        .flat_map(|(province, full, pinyin)| {
            [
                (*province, *full, *province, *pinyin, "省级"),
                (*province, *full, *full, *pinyin, "省级"),
            ]
        })
        .chain(
            CITIES
                .iter()
                .map(|(province, full, city, pinyin)| (*province, *full, *city, *pinyin, "地市")),
        )
        .collect()
}

const PROVINCES: &[(&str, &str, &str)] = &[
    ("北京", "北京市", "beijing"),
    ("天津", "天津市", "tianjin"),
    ("河北", "河北省", "hebei"),
    ("山西", "山西省", "shanxi"),
    ("内蒙古", "内蒙古自治区", "neimenggu"),
    ("辽宁", "辽宁省", "liaoning"),
    ("吉林", "吉林省", "jilin"),
    ("黑龙江", "黑龙江省", "heilongjiang"),
    ("上海", "上海市", "shanghai"),
    ("江苏", "江苏省", "jiangsu"),
    ("浙江", "浙江省", "zhejiang"),
    ("安徽", "安徽省", "anhui"),
    ("福建", "福建省", "fujian"),
    ("江西", "江西省", "jiangxi"),
    ("山东", "山东省", "shandong"),
    ("河南", "河南省", "henan"),
    ("湖北", "湖北省", "hubei"),
    ("湖南", "湖南省", "hunan"),
    ("广东", "广东省", "guangdong"),
    ("广西", "广西壮族自治区", "guangxi"),
    ("海南", "海南省", "hainan"),
    ("重庆", "重庆市", "chongqing"),
    ("四川", "四川省", "sichuan"),
    ("贵州", "贵州省", "guizhou"),
    ("云南", "云南省", "yunnan"),
    ("西藏", "西藏自治区", "xizang"),
    ("陕西", "陕西省", "shaanxi"),
    ("甘肃", "甘肃省", "gansu"),
    ("青海", "青海省", "qinghai"),
    ("宁夏", "宁夏回族自治区", "ningxia"),
    ("新疆", "新疆维吾尔自治区", "xinjiang"),
    ("台湾", "台湾省", "taiwan"),
    ("香港", "香港特别行政区", "xianggang"),
    ("澳门", "澳门特别行政区", "aomen"),
];

// Complete 364-city/重点单元 aliases copied from the baseline catalog.  Keeping
// this data in the Rust engine makes the report deterministic and offline.
const CITIES: &[(&str, &str, &str, &str)] = &[
    ("北京", "北京市", "北京", "beijing"),
    ("天津", "天津市", "天津", "tianjin"),
    ("河北", "河北省", "石家庄", "shijiazhuang"),
    ("河北", "河北省", "唐山", "tangshan"),
    ("河北", "河北省", "秦皇岛", "qinhuangdao"),
    ("河北", "河北省", "邯郸", "handan"),
    ("河北", "河北省", "邢台", "xingtai"),
    ("河北", "河北省", "保定", "baoding"),
    ("河北", "河北省", "张家口", "zhangjiakou"),
    ("河北", "河北省", "承德", "chengde"),
    ("河北", "河北省", "沧州", "cangzhou"),
    ("河北", "河北省", "廊坊", "langfang"),
    ("河北", "河北省", "衡水", "hengshui"),
    ("山西", "山西省", "太原", "taiyuan"),
    ("山西", "山西省", "大同", "datong"),
    ("山西", "山西省", "阳泉", "yangquan"),
    ("山西", "山西省", "长治", "changzhi"),
    ("山西", "山西省", "晋城", "jincheng"),
    ("山西", "山西省", "朔州", "shuozhou"),
    ("山西", "山西省", "晋中", "jinzhong"),
    ("山西", "山西省", "运城", "yuncheng"),
    ("山西", "山西省", "忻州", "xinzhou"),
    ("山西", "山西省", "临汾", "linfen"),
    ("山西", "山西省", "吕梁", "lvliang"),
    ("内蒙古", "内蒙古自治区", "呼和浩特", "huhehaote"),
    ("内蒙古", "内蒙古自治区", "包头", "baotou"),
    ("内蒙古", "内蒙古自治区", "乌海", "wuhai"),
    ("内蒙古", "内蒙古自治区", "赤峰", "chifeng"),
    ("内蒙古", "内蒙古自治区", "通辽", "tongliao"),
    ("内蒙古", "内蒙古自治区", "鄂尔多斯", "eerduosi"),
    ("内蒙古", "内蒙古自治区", "呼伦贝尔", "hulunbeier"),
    ("内蒙古", "内蒙古自治区", "巴彦淖尔", "bayannaoer"),
    ("内蒙古", "内蒙古自治区", "乌兰察布", "wulanchabu"),
    ("内蒙古", "内蒙古自治区", "兴安盟", "xinganmeng"),
    ("内蒙古", "内蒙古自治区", "锡林郭勒盟", "xilinguolemeng"),
    ("内蒙古", "内蒙古自治区", "阿拉善盟", "alashanmeng"),
    ("辽宁", "辽宁省", "沈阳", "shenyang"),
    ("辽宁", "辽宁省", "大连", "dalian"),
    ("辽宁", "辽宁省", "鞍山", "anshan"),
    ("辽宁", "辽宁省", "抚顺", "fushun"),
    ("辽宁", "辽宁省", "本溪", "benxi"),
    ("辽宁", "辽宁省", "丹东", "dandong"),
    ("辽宁", "辽宁省", "锦州", "jinzhou"),
    ("辽宁", "辽宁省", "营口", "yingkou"),
    ("辽宁", "辽宁省", "阜新", "fuxin"),
    ("辽宁", "辽宁省", "辽阳", "liaoyang"),
    ("辽宁", "辽宁省", "盘锦", "panjin"),
    ("辽宁", "辽宁省", "铁岭", "tieling"),
    ("辽宁", "辽宁省", "朝阳", "chaoyang"),
    ("辽宁", "辽宁省", "葫芦岛", "huludao"),
    ("吉林", "吉林省", "长春", "changchun"),
    ("吉林", "吉林省", "吉林", "jilin"),
    ("吉林", "吉林省", "四平", "siping"),
    ("吉林", "吉林省", "辽源", "liaoyuan"),
    ("吉林", "吉林省", "通化", "tonghua"),
    ("吉林", "吉林省", "白山", "baishan"),
    ("吉林", "吉林省", "松原", "songyuan"),
    ("吉林", "吉林省", "白城", "baicheng"),
    ("吉林", "吉林省", "延边", "yanbian"),
    ("黑龙江", "黑龙江省", "哈尔滨", "haerbin"),
    ("黑龙江", "黑龙江省", "齐齐哈尔", "qiqihaer"),
    ("黑龙江", "黑龙江省", "鸡西", "jixi"),
    ("黑龙江", "黑龙江省", "鹤岗", "hegang"),
    ("黑龙江", "黑龙江省", "双鸭山", "shuangyashan"),
    ("黑龙江", "黑龙江省", "大庆", "daqing"),
    ("黑龙江", "黑龙江省", "伊春", "yichun"),
    ("黑龙江", "黑龙江省", "佳木斯", "jiamusi"),
    ("黑龙江", "黑龙江省", "七台河", "qitaihe"),
    ("黑龙江", "黑龙江省", "牡丹江", "mudanjiang"),
    ("黑龙江", "黑龙江省", "黑河", "heihe"),
    ("黑龙江", "黑龙江省", "绥化", "suihua"),
    ("黑龙江", "黑龙江省", "大兴安岭", "daxinganling"),
    ("上海", "上海市", "上海", "shanghai"),
    ("江苏", "江苏省", "南京", "nanjing"),
    ("江苏", "江苏省", "无锡", "wuxi"),
    ("江苏", "江苏省", "徐州", "xuzhou"),
    ("江苏", "江苏省", "常州", "changzhou"),
    ("江苏", "江苏省", "苏州", "suzhou"),
    ("江苏", "江苏省", "南通", "nantong"),
    ("江苏", "江苏省", "连云港", "lianyungang"),
    ("江苏", "江苏省", "淮安", "huaian"),
    ("江苏", "江苏省", "盐城", "yancheng"),
    ("江苏", "江苏省", "扬州", "yangzhou"),
    ("江苏", "江苏省", "镇江", "zhenjiang"),
    ("江苏", "江苏省", "泰州", "taizhou"),
    ("江苏", "江苏省", "宿迁", "suqian"),
    ("浙江", "浙江省", "杭州", "hangzhou"),
    ("浙江", "浙江省", "宁波", "ningbo"),
    ("浙江", "浙江省", "温州", "wenzhou"),
    ("浙江", "浙江省", "嘉兴", "jiaxing"),
    ("浙江", "浙江省", "湖州", "huzhou"),
    ("浙江", "浙江省", "绍兴", "shaoxing"),
    ("浙江", "浙江省", "金华", "jinhua"),
    ("浙江", "浙江省", "衢州", "quzhou"),
    ("浙江", "浙江省", "舟山", "zhoushan"),
    ("浙江", "浙江省", "台州", "taizhou"),
    ("浙江", "浙江省", "丽水", "lishui"),
    ("安徽", "安徽省", "合肥", "hefei"),
    ("安徽", "安徽省", "芜湖", "wuhu"),
    ("安徽", "安徽省", "蚌埠", "bengbu"),
    ("安徽", "安徽省", "淮南", "huainan"),
    ("安徽", "安徽省", "马鞍山", "maanshan"),
    ("安徽", "安徽省", "淮北", "huaibei"),
    ("安徽", "安徽省", "铜陵", "tongling"),
    ("安徽", "安徽省", "安庆", "anqing"),
    ("安徽", "安徽省", "黄山", "huangshan"),
    ("安徽", "安徽省", "滁州", "chuzhou"),
    ("安徽", "安徽省", "阜阳", "fuyang"),
    ("安徽", "安徽省", "宿州", "suzhou"),
    ("安徽", "安徽省", "六安", "luan"),
    ("安徽", "安徽省", "亳州", "bozhou"),
    ("安徽", "安徽省", "池州", "chizhou"),
    ("安徽", "安徽省", "宣城", "xuancheng"),
    ("福建", "福建省", "福州", "fuzhou"),
    ("福建", "福建省", "厦门", "xiamen"),
    ("福建", "福建省", "莆田", "putian"),
    ("福建", "福建省", "三明", "sanming"),
    ("福建", "福建省", "泉州", "quanzhou"),
    ("福建", "福建省", "漳州", "zhangzhou"),
    ("福建", "福建省", "南平", "nanping"),
    ("福建", "福建省", "龙岩", "longyan"),
    ("福建", "福建省", "宁德", "ningde"),
    ("江西", "江西省", "南昌", "nanchang"),
    ("江西", "江西省", "景德镇", "jingdezhen"),
    ("江西", "江西省", "萍乡", "pingxiang"),
    ("江西", "江西省", "九江", "jiujiang"),
    ("江西", "江西省", "新余", "xinyu"),
    ("江西", "江西省", "鹰潭", "yingtan"),
    ("江西", "江西省", "赣州", "ganzhou"),
    ("江西", "江西省", "吉安", "jian"),
    ("江西", "江西省", "宜春", "yichun"),
    ("江西", "江西省", "抚州", "fuzhou"),
    ("江西", "江西省", "上饶", "shangrao"),
    ("山东", "山东省", "济南", "jinan"),
    ("山东", "山东省", "青岛", "qingdao"),
    ("山东", "山东省", "淄博", "zibo"),
    ("山东", "山东省", "枣庄", "zaozhuang"),
    ("山东", "山东省", "东营", "dongying"),
    ("山东", "山东省", "烟台", "yantai"),
    ("山东", "山东省", "潍坊", "weifang"),
    ("山东", "山东省", "济宁", "jining"),
    ("山东", "山东省", "泰安", "taian"),
    ("山东", "山东省", "威海", "weihai"),
    ("山东", "山东省", "日照", "rizhao"),
    ("山东", "山东省", "临沂", "linyi"),
    ("山东", "山东省", "德州", "dezhou"),
    ("山东", "山东省", "聊城", "liaocheng"),
    ("山东", "山东省", "滨州", "binzhou"),
    ("山东", "山东省", "菏泽", "heze"),
    ("河南", "河南省", "郑州", "zhengzhou"),
    ("河南", "河南省", "开封", "kaifeng"),
    ("河南", "河南省", "洛阳", "luoyang"),
    ("河南", "河南省", "平顶山", "pingdingshan"),
    ("河南", "河南省", "安阳", "anyang"),
    ("河南", "河南省", "鹤壁", "hebi"),
    ("河南", "河南省", "新乡", "xinxiang"),
    ("河南", "河南省", "焦作", "jiaozuo"),
    ("河南", "河南省", "濮阳", "puyang"),
    ("河南", "河南省", "许昌", "xuchang"),
    ("河南", "河南省", "漯河", "luohe"),
    ("河南", "河南省", "三门峡", "sanmenxia"),
    ("河南", "河南省", "南阳", "nanyang"),
    ("河南", "河南省", "商丘", "shangqiu"),
    ("河南", "河南省", "信阳", "xinyang"),
    ("河南", "河南省", "周口", "zhoukou"),
    ("河南", "河南省", "驻马店", "zhumadian"),
    ("河南", "河南省", "济源", "jiyuan"),
    ("湖北", "湖北省", "武汉", "wuhan"),
    ("湖北", "湖北省", "黄石", "huangshi"),
    ("湖北", "湖北省", "十堰", "shiyan"),
    ("湖北", "湖北省", "宜昌", "yichang"),
    ("湖北", "湖北省", "襄阳", "xiangyang"),
    ("湖北", "湖北省", "鄂州", "ezhou"),
    ("湖北", "湖北省", "荆门", "jingmen"),
    ("湖北", "湖北省", "孝感", "xiaogan"),
    ("湖北", "湖北省", "荆州", "jingzhou"),
    ("湖北", "湖北省", "黄冈", "huanggang"),
    ("湖北", "湖北省", "咸宁", "xianning"),
    ("湖北", "湖北省", "随州", "suizhou"),
    ("湖北", "湖北省", "恩施", "enshi"),
    ("湖北", "湖北省", "仙桃", "xiantao"),
    ("湖北", "湖北省", "潜江", "qianjiang"),
    ("湖北", "湖北省", "天门", "tianmen"),
    ("湖北", "湖北省", "神农架", "shennongjia"),
    ("湖南", "湖南省", "长沙", "changsha"),
    ("湖南", "湖南省", "株洲", "zhuzhou"),
    ("湖南", "湖南省", "湘潭", "xiangtan"),
    ("湖南", "湖南省", "衡阳", "hengyang"),
    ("湖南", "湖南省", "邵阳", "shaoyang"),
    ("湖南", "湖南省", "岳阳", "yueyang"),
    ("湖南", "湖南省", "常德", "changde"),
    ("湖南", "湖南省", "张家界", "zhangjiajie"),
    ("湖南", "湖南省", "益阳", "yiyang"),
    ("湖南", "湖南省", "郴州", "chenzhou"),
    ("湖南", "湖南省", "永州", "yongzhou"),
    ("湖南", "湖南省", "怀化", "huaihua"),
    ("湖南", "湖南省", "娄底", "loudi"),
    ("湖南", "湖南省", "湘西", "xiangxi"),
    ("广东", "广东省", "广州", "guangzhou"),
    ("广东", "广东省", "韶关", "shaoguan"),
    ("广东", "广东省", "深圳", "shenzhen"),
    ("广东", "广东省", "珠海", "zhuhai"),
    ("广东", "广东省", "汕头", "shantou"),
    ("广东", "广东省", "佛山", "foshan"),
    ("广东", "广东省", "江门", "jiangmen"),
    ("广东", "广东省", "湛江", "zhanjiang"),
    ("广东", "广东省", "茂名", "maoming"),
    ("广东", "广东省", "肇庆", "zhaoqing"),
    ("广东", "广东省", "惠州", "huizhou"),
    ("广东", "广东省", "梅州", "meizhou"),
    ("广东", "广东省", "汕尾", "shanwei"),
    ("广东", "广东省", "河源", "heyuan"),
    ("广东", "广东省", "阳江", "yangjiang"),
    ("广东", "广东省", "清远", "qingyuan"),
    ("广东", "广东省", "东莞", "dongguan"),
    ("广东", "广东省", "中山", "zhongshan"),
    ("广东", "广东省", "潮州", "chaozhou"),
    ("广东", "广东省", "揭阳", "jieyang"),
    ("广东", "广东省", "云浮", "yunfu"),
    ("广西", "广西壮族自治区", "南宁", "nanning"),
    ("广西", "广西壮族自治区", "柳州", "liuzhou"),
    ("广西", "广西壮族自治区", "桂林", "guilin"),
    ("广西", "广西壮族自治区", "梧州", "wuzhou"),
    ("广西", "广西壮族自治区", "北海", "beihai"),
    ("广西", "广西壮族自治区", "防城港", "fangchenggang"),
    ("广西", "广西壮族自治区", "钦州", "qinzhou"),
    ("广西", "广西壮族自治区", "贵港", "guigang"),
    ("广西", "广西壮族自治区", "玉林", "yulin"),
    ("广西", "广西壮族自治区", "百色", "baise"),
    ("广西", "广西壮族自治区", "贺州", "hezhou"),
    ("广西", "广西壮族自治区", "河池", "hechi"),
    ("广西", "广西壮族自治区", "来宾", "laibin"),
    ("广西", "广西壮族自治区", "崇左", "chongzuo"),
    ("海南", "海南省", "海口", "haikou"),
    ("海南", "海南省", "三亚", "sanya"),
    ("海南", "海南省", "三沙", "sansha"),
    ("海南", "海南省", "儋州", "danzhou"),
    ("重庆", "重庆市", "重庆", "chongqing"),
    ("四川", "四川省", "成都", "chengdu"),
    ("四川", "四川省", "自贡", "zigong"),
    ("四川", "四川省", "攀枝花", "panzhihua"),
    ("四川", "四川省", "泸州", "luzhou"),
    ("四川", "四川省", "德阳", "deyang"),
    ("四川", "四川省", "绵阳", "mianyang"),
    ("四川", "四川省", "广元", "guangyuan"),
    ("四川", "四川省", "遂宁", "suining"),
    ("四川", "四川省", "内江", "neijiang"),
    ("四川", "四川省", "乐山", "leshan"),
    ("四川", "四川省", "南充", "nanchong"),
    ("四川", "四川省", "眉山", "meishan"),
    ("四川", "四川省", "宜宾", "yibin"),
    ("四川", "四川省", "广安", "guangan"),
    ("四川", "四川省", "达州", "dazhou"),
    ("四川", "四川省", "雅安", "yaan"),
    ("四川", "四川省", "巴中", "bazhong"),
    ("四川", "四川省", "资阳", "ziyang"),
    ("四川", "四川省", "阿坝", "aba"),
    ("四川", "四川省", "甘孜", "ganzi"),
    ("四川", "四川省", "凉山", "liangshan"),
    ("贵州", "贵州省", "贵阳", "guiyang"),
    ("贵州", "贵州省", "六盘水", "liupanshui"),
    ("贵州", "贵州省", "遵义", "zunyi"),
    ("贵州", "贵州省", "安顺", "anshun"),
    ("贵州", "贵州省", "毕节", "bijie"),
    ("贵州", "贵州省", "铜仁", "tongren"),
    ("贵州", "贵州省", "黔西南", "qianxinan"),
    ("贵州", "贵州省", "黔东南", "qiandongnan"),
    ("贵州", "贵州省", "黔南", "qiannan"),
    ("云南", "云南省", "昆明", "kunming"),
    ("云南", "云南省", "曲靖", "qujing"),
    ("云南", "云南省", "玉溪", "yuxi"),
    ("云南", "云南省", "保山", "baoshan"),
    ("云南", "云南省", "昭通", "zhaotong"),
    ("云南", "云南省", "丽江", "lijiang"),
    ("云南", "云南省", "普洱", "puer"),
    ("云南", "云南省", "临沧", "lincang"),
    ("云南", "云南省", "楚雄", "chuxiong"),
    ("云南", "云南省", "红河", "honghe"),
    ("云南", "云南省", "文山", "wenshan"),
    ("云南", "云南省", "西双版纳", "xishuangbanna"),
    ("云南", "云南省", "大理", "dali"),
    ("云南", "云南省", "德宏", "dehong"),
    ("云南", "云南省", "怒江", "nujiang"),
    ("云南", "云南省", "迪庆", "diqing"),
    ("西藏", "西藏自治区", "拉萨", "lasa"),
    ("西藏", "西藏自治区", "日喀则", "rikaze"),
    ("西藏", "西藏自治区", "昌都", "changdu"),
    ("西藏", "西藏自治区", "林芝", "linzhi"),
    ("西藏", "西藏自治区", "山南", "shannan"),
    ("西藏", "西藏自治区", "那曲", "naqu"),
    ("西藏", "西藏自治区", "阿里", "ali"),
    ("陕西", "陕西省", "西安", "xian"),
    ("陕西", "陕西省", "铜川", "tongchuan"),
    ("陕西", "陕西省", "宝鸡", "baoji"),
    ("陕西", "陕西省", "咸阳", "xianyang"),
    ("陕西", "陕西省", "渭南", "weinan"),
    ("陕西", "陕西省", "延安", "yanan"),
    ("陕西", "陕西省", "汉中", "hanzhong"),
    ("陕西", "陕西省", "榆林", "yulin"),
    ("陕西", "陕西省", "安康", "ankang"),
    ("陕西", "陕西省", "商洛", "shangluo"),
    ("甘肃", "甘肃省", "兰州", "lanzhou"),
    ("甘肃", "甘肃省", "嘉峪关", "jiayuguan"),
    ("甘肃", "甘肃省", "金昌", "jinchang"),
    ("甘肃", "甘肃省", "白银", "baiyin"),
    ("甘肃", "甘肃省", "天水", "tianshui"),
    ("甘肃", "甘肃省", "武威", "wuwei"),
    ("甘肃", "甘肃省", "张掖", "zhangye"),
    ("甘肃", "甘肃省", "平凉", "pingliang"),
    ("甘肃", "甘肃省", "酒泉", "jiuquan"),
    ("甘肃", "甘肃省", "庆阳", "qingyang"),
    ("甘肃", "甘肃省", "定西", "dingxi"),
    ("甘肃", "甘肃省", "陇南", "longnan"),
    ("甘肃", "甘肃省", "临夏", "linxia"),
    ("甘肃", "甘肃省", "甘南", "gannan"),
    ("青海", "青海省", "西宁", "xining"),
    ("青海", "青海省", "海东", "haidong"),
    ("青海", "青海省", "海北", "haibei"),
    ("青海", "青海省", "黄南", "huangnan"),
    ("青海", "青海省", "海南州", "hainanzhou"),
    ("青海", "青海省", "果洛", "guoluo"),
    ("青海", "青海省", "玉树", "yushu"),
    ("青海", "青海省", "海西", "haixi"),
    ("宁夏", "宁夏回族自治区", "银川", "yinchuan"),
    ("宁夏", "宁夏回族自治区", "石嘴山", "shizuishan"),
    ("宁夏", "宁夏回族自治区", "吴忠", "wuzhong"),
    ("宁夏", "宁夏回族自治区", "固原", "guyuan"),
    ("宁夏", "宁夏回族自治区", "中卫", "zhongwei"),
    ("新疆", "新疆维吾尔自治区", "乌鲁木齐", "wulumuqi"),
    ("新疆", "新疆维吾尔自治区", "克拉玛依", "kelamayi"),
    ("新疆", "新疆维吾尔自治区", "吐鲁番", "tulufan"),
    ("新疆", "新疆维吾尔自治区", "哈密", "hami"),
    ("新疆", "新疆维吾尔自治区", "昌吉", "changji"),
    ("新疆", "新疆维吾尔自治区", "博尔塔拉", "boertala"),
    ("新疆", "新疆维吾尔自治区", "巴音郭楞", "bayinguoleng"),
    ("新疆", "新疆维吾尔自治区", "阿克苏", "akesu"),
    ("新疆", "新疆维吾尔自治区", "克孜勒苏", "kezilesu"),
    ("新疆", "新疆维吾尔自治区", "喀什", "kashi"),
    ("新疆", "新疆维吾尔自治区", "和田", "hetian"),
    ("新疆", "新疆维吾尔自治区", "伊犁", "yili"),
    ("新疆", "新疆维吾尔自治区", "塔城", "tacheng"),
    ("新疆", "新疆维吾尔自治区", "阿勒泰", "aletai"),
    ("新疆", "新疆维吾尔自治区", "石河子", "shihezi"),
    ("新疆", "新疆维吾尔自治区", "阿拉尔", "alaer"),
    ("新疆", "新疆维吾尔自治区", "图木舒克", "tumushuke"),
    ("新疆", "新疆维吾尔自治区", "五家渠", "wujiaqu"),
    ("新疆", "新疆维吾尔自治区", "北屯", "beitun"),
    ("新疆", "新疆维吾尔自治区", "铁门关", "tiemenguan"),
    ("新疆", "新疆维吾尔自治区", "双河", "shuanghe"),
    ("新疆", "新疆维吾尔自治区", "可克达拉", "kekedala"),
    ("新疆", "新疆维吾尔自治区", "昆玉", "kunyu"),
    ("新疆", "新疆维吾尔自治区", "胡杨河", "huyanghe"),
    ("新疆", "新疆维吾尔自治区", "新星", "xinxing"),
    ("台湾", "台湾省", "台北", "taibei"),
    ("台湾", "台湾省", "新北", "xinbei"),
    ("台湾", "台湾省", "桃园", "taoyuan"),
    ("台湾", "台湾省", "台中", "taizhong"),
    ("台湾", "台湾省", "台南", "tainan"),
    ("台湾", "台湾省", "高雄", "gaoxiong"),
    ("台湾", "台湾省", "基隆", "jilong"),
    ("台湾", "台湾省", "新竹", "xinzhu"),
    ("台湾", "台湾省", "嘉义", "jiayi"),
    ("香港", "香港特别行政区", "香港", "xianggang"),
    ("澳门", "澳门特别行政区", "澳门", "aomen"),
];

fn contains_alias(text: &str, alias: &str) -> bool {
    if alias.len() < 2 {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    let alias = alias.to_ascii_lowercase();
    if !alias
        .chars()
        .all(|character| character.is_ascii_alphabetic())
    {
        return lower.contains(&alias);
    }
    let mut offset = 0;
    while let Some(index) = lower[offset..].find(&alias) {
        let start = offset + index;
        let end = start + alias.len();
        let before = lower[..start].chars().next_back();
        let after = lower[end..].chars().next();
        if !before.is_some_and(|character| character.is_ascii_alphabetic())
            && !after.is_some_and(|character| character.is_ascii_alphabetic())
        {
            return true;
        }
        offset = end;
    }
    false
}

fn footprint_category(value: &str, host: &str, identity: bool) -> &'static str {
    if identity {
        return "客户/租户标识";
    }
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("jdbc:")
        || lower.starts_with("mongodb")
        || lower.starts_with("redis")
        || lower.starts_with("amqp")
    {
        return "数据库/中间件";
    }
    if lower.starts_with("ws://") || lower.starts_with("wss://") {
        return "WebSocket";
    }
    if lower.starts_with("ftp://") || lower.starts_with("sftp://") {
        return "文件传输";
    }
    if [
        "s3",
        "oss-",
        "cos.",
        "myqcloud",
        "obs.",
        "minio",
        "blob.core.windows.net",
        "storage.googleapis.com",
    ]
    .iter()
    .any(|word| lower.contains(word) || host.contains(word))
    {
        return "对象存储/CDN";
    }
    if lower.contains("callback") || lower.contains("webhook") {
        return "回调/Webhook";
    }
    if host.parse::<std::net::IpAddr>().is_ok() {
        return "IP 地址";
    }
    "URL/API 地址"
}

fn footprint_risk(value: &str, host: &str, path: &str, category: &str) -> (&'static str, u64) {
    let common_hosts = [
        "w3.org",
        "schema.org",
        "github.com",
        "npmjs.org",
        "registry.npmjs.org",
        "pypi.org",
        "maven.apache.org",
        "gradle.org",
        "example.com",
        "localhost",
    ];
    if common_hosts
        .iter()
        .any(|item| host == *item || host.ends_with(&format!(".{item}")))
    {
        return ("提示", 0);
    }
    let mut score = 0_u64;
    if category == "客户/租户标识" {
        score += 3;
    }
    if category == "数据库/中间件" {
        score += 5;
    }
    if matches!(category, "文件传输" | "回调/Webhook") {
        score += 3;
    }
    if matches!(category, "对象存储/CDN" | "WebSocket") {
        score += 2;
    }
    if host.parse::<std::net::IpAddr>().is_ok() || (!host.is_empty() && host != "localhost") {
        score += 2;
    }
    let context = format!("{path} {value}").to_ascii_lowercase();
    if [
        "old",
        "legacy",
        "dev",
        "test",
        "stage",
        "uat",
        "prod",
        "production",
    ]
    .iter()
    .any(|word| context.contains(word))
    {
        score += 2;
    }
    if [
        "config",
        "application",
        "setting",
        ".env",
        "properties",
        "yaml",
        "yml",
        "toml",
        "xml",
    ]
    .iter()
    .any(|word| path.to_ascii_lowercase().contains(word))
    {
        score += 1;
    }
    if score >= 5 {
        ("高", score)
    } else if score >= 3 {
        ("中", score)
    } else if score > 0 {
        ("低", score)
    } else {
        ("提示", score)
    }
}

fn host(value: &str) -> String {
    if let Ok(url) = Url::parse(value) {
        return url.host_str().unwrap_or_default().to_ascii_lowercase();
    }
    value
        .strip_prefix("jdbc:")
        .unwrap_or(value)
        .split("//")
        .nth(1)
        .unwrap_or(value)
        .split(['/', ':'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn redact_value(value: &str) -> String {
    let mut value = clean_text(value);
    if let Ok(mut url) = Url::parse(&value) {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        value = url.to_string();
    }
    redact_credentials(&value)
}

fn redact_text(value: &str) -> String {
    redact_credentials(&value.replace('\0', " "))
}

fn redact_credentials(value: &str) -> String {
    static URL_USERINFO: OnceLock<Regex> = OnceLock::new();
    static JDBC_USERINFO: OnceLock<Regex> = OnceLock::new();
    static BEARER: OnceLock<Regex> = OnceLock::new();
    static SECRET: OnceLock<Regex> = OnceLock::new();
    let value = URL_USERINFO
        .get_or_init(|| {
            Regex::new(r#"(?i)(\b[a-z][a-z0-9+.-]*://)[^/\s:@]+(?::[^/\s@]*)?@"#)
                .expect("valid URL credential regex")
        })
        .replace_all(value, "$1[已脱敏]@");
    let value = JDBC_USERINFO
        .get_or_init(|| {
            Regex::new(r#"(?i)(\bjdbc:[^:\s]+(?::[^:\s]+)*:)[^/\s:@]+/[^@\s]+@"#)
                .expect("valid JDBC credential regex")
        })
        .replace_all(&value, "$1[已脱敏]@");
    let value = BEARER
        .get_or_init(|| {
            Regex::new(r#"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+"#)
                .expect("valid bearer credential regex")
        })
        .replace_all(&value, "Bearer [已脱敏]");
    SECRET
        .get_or_init(|| {
            Regex::new(
                r#"(?i)["']?(user(?:name)?|login|password|passwd|pwd|token|secret|api[_-]?key|access[_-]?token|client[_-]?secret|authorization|private[_-]?key)["']?\s*([:=])\s*(?:"[^"]*"|'[^']*'|[^,;\s}\]]+)"#,
            )
            .expect("valid redaction regex")
        })
        .replace_all(&value, "$1$2[已脱敏]")
        .into_owned()
}

fn clean_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| !matches!(character, '\r' | '\n' | '\0'))
        .collect::<String>()
        .trim()
        .to_owned()
}

fn split_requirement(value: &str) -> (&str, &str) {
    let value = value.split_once(';').map_or(value, |(head, _)| head).trim();
    let index = value
        .find(['<', '>', '=', '!', '~', ' '])
        .unwrap_or(value.len());
    let name = value[..index].split('[').next().unwrap_or(&value[..index]);
    let version = value[index..].trim();
    (
        name,
        if version.is_empty() {
            "未指定"
        } else {
            version
        },
    )
}

fn toml_scalar(value: &toml::Value) -> String {
    match value {
        toml::Value::String(value) => value.clone(),
        toml::Value::Table(table) => table
            .get("version")
            .map(toml_scalar)
            .or_else(|| table.get("git").map(toml_scalar))
            .unwrap_or_else(|| "未指定".to_owned()),
        _ => value.to_string().trim_matches('"').to_owned(),
    }
}

fn is_unpinned(version: &str) -> bool {
    let version = version.trim().to_ascii_lowercase();
    version.is_empty()
        || matches!(version.as_str(), "*" | "latest" | "未指定" | "由父项目管理")
        || version.starts_with("git+")
        || version.starts_with("http://")
        || version.starts_with("https://")
        || version.starts_with('^')
        || version.starts_with('~')
        || version.starts_with('>')
        || version.starts_with('<')
}

fn line_for(content: &str, value: &str) -> u64 {
    content
        .lines()
        .position(|line| line.contains(value))
        .map_or(1, |line| (line + 1) as u64)
}

fn line_number(content: &str, line: &str) -> u64 {
    line_for(content, line.trim())
}

fn deduplicate_items(items: &mut Vec<EvidenceItem>) {
    let mut seen = BTreeSet::new();
    items.retain(|item| {
        seen.insert(format!(
            "{}\0{}\0{}",
            item.category,
            item.path.as_deref().unwrap_or_default(),
            item.value
        ))
    });
    items.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.value.cmp(&right.value))
    });
    items.truncate(MAX_EVIDENCE);
}

fn report(
    kind: &str,
    items: Vec<EvidenceItem>,
    scanned_files: u64,
    truncated: bool,
    limitation: &str,
) -> ReportSnapshot {
    ReportSnapshot {
        kind: kind.to_owned(),
        available: true,
        items,
        limitations: vec![limitation.to_owned()],
        score: None,
        level: None,
        conclusion: None,
        scanned_files,
        truncated,
    }
}

fn unavailable_report(kind: &str, limitation: &str) -> ReportSnapshot {
    ReportSnapshot {
        kind: kind.to_owned(),
        available: false,
        items: Vec::new(),
        limitations: vec![limitation.to_owned()],
        score: None,
        level: None,
        conclusion: None,
        scanned_files: 0,
        truncated: false,
    }
}

/// A delivery candidate kept as a reference into the history rows.  Keeping
/// the candidate private lets the wire model remain stable while the
/// report-specific metadata can grow without another database table.
struct DeliveryCandidate<'a> {
    commit: &'a CommitSummary,
    score: f64,
    semantic_score: u64,
    tagged: bool,
    tags: Vec<String>,
    added_ratio: f64,
    gap_to_next_days: Option<f64>,
    position: usize,
    reasons: Vec<String>,
}

fn delivery_report(commits: &[CommitSummary], refs: &[RefSummary]) -> ReportSnapshot {
    let keywords = [
        "initial", "import", "source", "delivery", "release", "baseline", "首次", "初始", "导入",
        "源码", "交付", "上线", "发布", "基线",
    ];
    let tagged_refs: Vec<(&str, String)> = refs
        .iter()
        .filter_map(|reference| {
            reference
                .name
                .strip_prefix("refs/tags/")
                .zip(reference.target.as_deref())
                .map(|(tag, target)| (target, redact_text(tag)))
        })
        .collect();
    let max_additions = commits.iter().map(|commit| commit.additions).max().unwrap_or(0);
    let max_files = commits
        .iter()
        .map(|commit| commit.files_changed)
        .max()
        .unwrap_or(0);
    let mut candidates: Vec<DeliveryCandidate<'_>> = commits
        .iter()
        .enumerate()
        .filter_map(|(position, commit)| {
            let lower = commit.subject.to_ascii_lowercase();
            let semantic_hits = keywords
                .iter()
                .filter(|keyword| lower.contains(**keyword))
                .count() as u64;
            let semantic_score = semantic_hits.saturating_mul(5).min(15);
            let tags = tagged_refs
                .iter()
                .filter(|(target, _)| *target == commit.oid.as_str())
                .map(|(_, tag)| tag.clone())
                .collect::<Vec<_>>();
            let tagged = !tags.is_empty();
            let churn = commit.additions.saturating_add(commit.deletions);
            let added_ratio = if churn == 0 {
                0.0
            } else {
                commit.additions as f64 * 100.0 / churn as f64
            };
            let mut score = if max_additions == 0 {
                0.0
            } else {
                35.0 * commit.additions as f64 / max_additions as f64
            };
            score += if max_files == 0 {
                0.0
            } else {
                25.0 * commit.files_changed as f64 / max_files as f64
            };
            score += 15.0 * added_ratio / 100.0;
            let mut reasons = Vec::new();
            if commit.additions > 0 {
                reasons.push(format!("新增 {} 行", commit.additions));
            }
            if commit.files_changed > 0 {
                reasons.push(format!("涉及 {} 个文件", commit.files_changed));
            }
            if semantic_score > 0 {
                score += semantic_score as f64;
                reasons.push("提交说明含导入/交付/发布语义".to_owned());
            }
            if has_version_signal(&lower) {
                score += 5.0;
                reasons.push("提交说明含版本号线索".to_owned());
            }
            if tagged {
                score += 10.0;
                reasons.push("提交对应可见版本标签".to_owned());
            }
            if commit.parent_count == 0 {
                score -= 25.0;
                reasons.push("项目初始化根提交，已降权".to_owned());
            } else if commit.parent_count > 1 {
                score -= 15.0;
                reasons.push("合并提交，已降权".to_owned());
            }
            if commit.additions == 0 && commit.deletions > 0 {
                score -= 10.0;
                reasons.push("以删除为主，已降权".to_owned());
            }
            let gap_to_next_days = gap_to_next_newer_commit(commits, position, commit.authored_at);
            if let Some(gap) = gap_to_next_days {
                if gap >= 180.0 {
                    score += 12.0;
                    reasons.push(format!("与下一次可见提交间隔 {:.1} 天", gap));
                } else if gap >= 30.0 {
                    score += 7.0;
                    reasons.push(format!("与下一次可见提交间隔 {:.1} 天", gap));
                }
            }
            score = score.clamp(0.0, 100.0);
            (score > 0.0 || semantic_score > 0 || tagged).then_some(DeliveryCandidate {
                commit,
                score,
                semantic_score,
                tagged,
                tags,
                added_ratio,
                gap_to_next_days,
                position,
                reasons,
            })
        })
        .collect();
    if candidates.is_empty()
        && let Some(commit) = commits.last()
    {
        candidates.push(DeliveryCandidate {
            commit,
            score: 1.0,
            semantic_score: 0,
            tagged: false,
            tags: Vec::new(),
            added_ratio: 0.0,
            gap_to_next_days: None,
            position: commits.len().saturating_sub(1),
            reasons: vec!["没有发现明显交付语义，保留最早可见提交供复核".to_owned()],
        });
    }
    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| right.commit.additions.cmp(&left.commit.additions))
            .then_with(|| right.commit.files_changed.cmp(&left.commit.files_changed))
            .then_with(|| left.position.cmp(&right.position))
    });
    candidates.truncate(12);
    let items = candidates
        .iter()
        .map(|candidate| EvidenceItem {
            category: "delivery".to_owned(),
            kind: "候选版本".to_owned(),
            value: format!("{} {}", candidate.commit.short_oid, candidate.commit.subject),
            path: None,
            line: None,
            conclusion: "这是交付溯源候选，不是交付事实认定。".to_owned(),
            metadata: Some(serde_json::json!({
                "oid": candidate.commit.oid,
                "authoredAt": candidate.commit.authored_at,
                "score": candidate.score,
                "additions": candidate.commit.additions,
                "deletions": candidate.commit.deletions,
                "filesChanged": candidate.commit.files_changed,
                "addedFileRatio": candidate.added_ratio,
                "semanticScore": candidate.semantic_score,
                "tagged": candidate.tagged,
                "tags": candidate.tags,
                "isRoot": candidate.commit.parent_count == 0,
                "isMerge": candidate.commit.parent_count > 1,
                "historyPosition": candidate.position + 1,
                "gapToNextDays": candidate.gap_to_next_days,
                "reasons": candidate.reasons,
            })),
        })
        .collect::<Vec<_>>();
    let tagged = tagged_refs.len();
    let best_score = candidates.first().map(|candidate| candidate.score);
    let (level, conclusion) = match best_score {
        Some(score) if score >= 70.0 => (
            Some("高".to_owned()),
            Some("候选节点呈现较强的成品式代码导入特征，建议重点核验交付前历史；这不是事实认定。".to_owned()),
        ),
        Some(score) if score >= 40.0 => (
            Some("中".to_owned()),
            Some("候选节点存在集中交付或代码导入特征，仍需结合原始仓库和合同里程碑核验。".to_owned()),
        ),
        Some(_) => (
            Some("低".to_owned()),
            Some("当前可见 Git 证据不足以支持一次性交付判断，仅保留候选节点供复核。".to_owned()),
        ),
        None => (None, Some("仓库没有可见提交，无法建立交付候选。".to_owned())),
    };
    ReportSnapshot {
        kind: "delivery".to_owned(),
        available: !items.is_empty(),
        items,
        limitations: vec![
            "交付候选综合提交规模、文件数、新增比例、语义、标签、时间间隔及根/合并降权排序；不等同于合同交付日期或来源事实。"
                .to_owned(),
            format!("可见标签数量：{tagged}；未访问远程服务。"),
            "当前批次以提交元数据建立候选；版本树中的文件、版权年份、构建产物和声明版本仍需打开对应提交逐项复核。"
                .to_owned(),
        ],
        score: best_score,
        level,
        conclusion,
        scanned_files: 0,
        truncated: false,
    }
}

fn has_version_signal(subject: &str) -> bool {
    let bytes = subject.as_bytes();
    bytes.windows(3).any(|window| {
        window[0] == b'v'
            && window[1].is_ascii_digit()
            && window[2..].iter().any(|byte| *byte == b'.' || byte.is_ascii_digit())
    }) || bytes.windows(3).any(|window| {
        window[0].is_ascii_digit() && window[1] == b'.' && window[2].is_ascii_digit()
    })
}

fn gap_to_next_newer_commit(
    commits: &[CommitSummary],
    position: usize,
    authored_at: i64,
) -> Option<f64> {
    commits
        .iter()
        .enumerate()
        .filter(|(index, commit)| *index != position && commit.authored_at > authored_at)
        .map(|(_, commit)| (commit.authored_at - authored_at) as f64 / 86_400_000.0)
        .min_by(|left, right| left.total_cmp(right))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_removed_from_urls_and_lines() {
        let value = redact_value("https://alice:pw@example.com/api?token=secret");
        assert!(!value.contains("alice"));
        assert!(!value.contains("secret"));
        let text = redact_text("username=alice password=abc Authorization: Bearer abc123");
        assert!(!text.contains("alice"));
        assert!(!text.contains("abc123"));
        assert!(text.matches("[已脱敏]").count() >= 3);
        assert!(
            !redact_value("jdbc:oracle:thin:alice/secret@//db.local:1521/service")
                .contains("secret")
        );
    }

    #[test]
    fn dependency_metadata_redacts_vcs_credentials() {
        let mut items = Vec::new();
        parse_package_json(
            "package.json",
            r#"{"dependencies":{"private":"git+https://alice:secret@example.com/pkg.git"}}"#,
            &mut items,
        );
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert!(!item.value.contains("alice"));
        assert!(!item.value.contains("secret"));
        assert!(!item.metadata.as_ref().unwrap().to_string().contains("secret"));
    }

    #[test]
    fn dependency_versions_mark_ranges_as_unpinned() {
        assert!(is_unpinned("^1.2.3"));
        assert!(!is_unpinned("1.2.3"));
    }

    #[test]
    fn ascii_region_aliases_need_word_boundaries() {
        assert!(contains_alias("deploy to beijing now", "beijing"));
        assert!(!contains_alias("beijingish", "beijing"));
    }

    #[test]
    fn region_catalog_keeps_the_full_baseline_size() {
        assert_eq!(PROVINCES.len(), 34);
        assert_eq!(CITIES.len(), 364);
    }

    #[test]
    fn delivery_candidates_keep_size_semantics_tags_and_initialization_penalty() {
        let commits = vec![
            CommitSummary {
                oid: "new".to_owned(),
                short_oid: "new".to_owned(),
                subject: "fix: follow-up".to_owned(),
                author_name: "a".to_owned(),
                author_email: "a@example.com".to_owned(),
                authored_at: 2 * 86_400_000,
                parent_count: 1,
                additions: 4,
                deletions: 1,
                files_changed: 1,
                category: "fix".to_owned(),
            },
            CommitSummary {
                oid: "delivery".to_owned(),
                short_oid: "deliv".to_owned(),
                subject: "导入交付源码 v1.2".to_owned(),
                author_name: "a".to_owned(),
                author_email: "a@example.com".to_owned(),
                authored_at: 86_400_000,
                parent_count: 1,
                additions: 100,
                deletions: 2,
                files_changed: 20,
                category: "other".to_owned(),
            },
            CommitSummary {
                oid: "root".to_owned(),
                short_oid: "root".to_owned(),
                subject: "initial import".to_owned(),
                author_name: "a".to_owned(),
                author_email: "a@example.com".to_owned(),
                authored_at: 0,
                parent_count: 0,
                additions: 100,
                deletions: 0,
                files_changed: 20,
                category: "other".to_owned(),
            },
        ];
        let refs = vec![RefSummary {
            name: "refs/tags/v1.2".to_owned(),
            raw_name_hex: None,
            target: Some("delivery".to_owned()),
            excluded: false,
        }];
        let report = delivery_report(&commits, &refs);
        assert!(report.available);
        assert_eq!(report.items.len(), 3);
        assert_eq!(report.level.as_deref(), Some("高"));
        let metadata = report.items[0].metadata.as_ref().unwrap().to_string();
        assert!(metadata.contains("filesChanged"));
        assert!(metadata.contains("semanticScore"));
        assert!(metadata.contains("tags"));
        assert!(report.score.unwrap_or_default() >= 70.0);
    }
}
