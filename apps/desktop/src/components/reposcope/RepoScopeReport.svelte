<script lang="ts">
	import { onMount } from "svelte";
	import RepoScopeNavigation from "$components/reposcope/RepoScopeNavigation.svelte";
	import { filePreviewHref, RepoScopeService } from "$lib/reposcope/service.svelte";
	import type {
		AnalysisStatus,
		AuthorSummary,
		BusFactorReport,
		CodeAgeBucket,
		CommitSummary,
		CouplingSummary,
		DailyActivity,
		DirectoryOwnership,
		FileSummary,
		LanguageSummary,
		Overview,
		OwnershipRow,
		Page,
		RefSummary,
		ReportSnapshot,
		RepositoryDiagnostics,
	} from "$lib/reposcope/types";

	const { projectId, report }: { projectId: string; report: string } = $props();
	const service = new RepoScopeService();

	let loading = $state(true);
	let error = $state<string>();
	let status = $state<AnalysisStatus>();
	let overview = $state<Overview>();
	let activity = $state<DailyActivity[]>([]);
	let commits = $state<Page<CommitSummary>>();
	let files = $state<Page<FileSummary>>();
	let authors = $state<Page<AuthorSummary>>();
	let languages = $state<LanguageSummary[]>([]);
	let couplings = $state<Page<CouplingSummary>>();
	let ownership = $state<Page<OwnershipRow>>();
	let directoryOwnership = $state<Page<DirectoryOwnership>>();
	let codeAge = $state<CodeAgeBucket[]>([]);
	let busFactor = $state<BusFactorReport>();
	let refs = $state<Page<RefSummary>>();
	let diagnostics = $state<RepositoryDiagnostics>();
	let evidence = $state<ReportSnapshot>();
	let pageNumber = $state(1);
	const pageSize = 50;

	const titles: Record<string, string> = {
		overview: "项目总览",
		activity: "活跃度",
		commits: "提交",
		files: "文件演化",
		authors: "贡献者",
		languages: "语言",
		hotspots: "热点文件",
		coupling: "变更耦合",
		ownership: "代码所有权",
		"directory-ownership": "目录所有权",
		"code-age": "代码年龄",
		"bus-factor": "Bus Factor",
		delivery: "交付溯源",
		footprints: "外部系统",
		regions: "地域线索",
		dependencies: "依赖与插件",
		diagnostics: "工作区状态",
		refs: "Refs / 标签",
	};

	type EvidenceReport = "delivery" | "footprints" | "regions" | "dependencies";
	function isEvidenceReport(value: string): value is EvidenceReport {
		return ["delivery", "footprints", "regions", "dependencies"].includes(value);
	}

	function isPagedReport(value: string) {
		return [
			"commits",
			"files",
			"hotspots",
			"authors",
			"coupling",
			"ownership",
			"directory-ownership",
			"refs",
		].includes(value);
	}

	const pageTotal = $derived(
		report === "commits"
			? (commits?.total ?? 0)
			: report === "files" || report === "hotspots"
				? (files?.total ?? 0)
				: report === "authors"
					? (authors?.total ?? 0)
					: report === "coupling"
						? (couplings?.total ?? 0)
						: report === "ownership"
							? (ownership?.total ?? 0)
							: report === "directory-ownership"
								? (directoryOwnership?.total ?? 0)
								: report === "refs"
									? (refs?.total ?? 0)
									: 0,
	);
	const pageCount = $derived(Math.max(1, Math.ceil(pageTotal / pageSize)));

	async function load() {
		loading = true;
		error = undefined;
		try {
			status = (await service.status(projectId)) ?? undefined;
			switch (report) {
				case "overview":
					overview = (await service.overview(projectId)) ?? undefined;
					break;
				case "activity":
					activity = await service.activity(projectId);
					break;
				case "commits":
					commits = await service.commits(projectId, pageNumber, pageSize);
					break;
				case "files":
					files = await service.fileEvolution(projectId, pageNumber, pageSize);
					break;
				case "authors":
					authors = await service.authors(projectId, pageNumber, pageSize);
					break;
				case "languages":
					languages = await service.languages(projectId);
					break;
				case "hotspots":
					files = await service.files(projectId, pageNumber, pageSize);
					break;
				case "coupling":
					couplings = await service.coupling(projectId, pageNumber, pageSize);
					break;
				case "ownership":
					ownership = await service.ownership(projectId, pageNumber, pageSize);
					break;
				case "directory-ownership":
					directoryOwnership = await service.directoryOwnership(projectId, pageNumber, pageSize);
					break;
				case "code-age":
					codeAge = await service.codeAge(projectId);
					break;
				case "bus-factor":
					busFactor = await service.busFactor(projectId);
					break;
				case "delivery":
				case "footprints":
				case "regions":
				case "dependencies":
					evidence = await service.report(projectId, report as EvidenceReport);
					break;
				case "diagnostics":
					diagnostics = await service.diagnostics(projectId);
					break;
				case "refs":
					refs = await service.refs(projectId, pageNumber, pageSize);
					break;
				default:
					error = "未识别的洞察页面";
			}
		} catch (cause) {
			error = cause instanceof Error ? cause.message : "读取洞察数据失败";
		} finally {
			loading = false;
		}
	}

	function changePage(delta: number) {
		const next = Math.min(pageCount, Math.max(1, pageNumber + delta));
		if (next === pageNumber) return;
		pageNumber = next;
		void load();
	}

	onMount(() => {
		void load();
	});
</script>

<div class="report-shell">
	<RepoScopeNavigation />
	<header class="report-header">
		<div>
			<a class="back" href={`/${projectId}/insights`}>← 返回总览</a>
			<h1>{titles[report] ?? "仓库洞察"}</h1>
			<p>只读取本机分析数据库；地域、交付和外部系统结果均为待核验线索。</p>
		</div>
		{#if status}<span class="status">{status.status}</span>{/if}
	</header>

	{#if error}<div class="error">{error}</div>{/if}
	{#if loading}
		<div class="empty">正在读取分析结果…</div>
	{:else if report === "overview" && overview}
		<div class="cards">
			<div><span>提交</span><b>{overview.commitCount.toLocaleString()}</b></div>
			<div><span>文件</span><b>{overview.fileCount.toLocaleString()}</b></div>
			<div><span>代码行</span><b>{overview.lines.toLocaleString()}</b></div>
			<div><span>贡献者</span><b>{overview.authorCount.toLocaleString()}</b></div>
		</div>
	{:else if report === "activity"}
		<section class="panel"><table><thead><tr><th>UTC 日期</th><th>提交</th><th>新增</th><th>删除</th><th>活跃作者</th></tr></thead><tbody>{#each activity as row}<tr><td>{row.date}</td><td>{row.commits}</td><td class="positive">+{row.additions}</td><td class="negative">-{row.deletions}</td><td>{row.activeAuthors}</td></tr>{/each}</tbody></table></section>
	{:else if report === "commits" && commits}
		<section class="panel"><div class="count">共 {commits.total} 条</div><table><thead><tr><th>OID</th><th>主题</th><th>作者</th><th>分类</th><th>增删</th></tr></thead><tbody>{#each commits.items as row}<tr><td><a href={`/${projectId}/insights/commit/${row.oid}`}><code>{row.shortOid}</code></a></td><td>{row.subject || "（无标题）"}</td><td>{row.authorName}</td><td>{row.category}</td><td class="delta">+{row.additions} / -{row.deletions}</td></tr>{/each}</tbody></table></section>
	{:else if (report === "files" || report === "hotspots") && files}
		<section class="panel"><div class="count">共 {files.total} 个文件</div><table><thead><tr><th>文件</th><th>语言</th><th>状态</th><th>代码行</th><th>热点</th></tr></thead><tbody>{#each files.items as row}<tr><td title={row.path}><a href={filePreviewHref(projectId, row.path)}>{row.path}</a></td><td>{row.language ?? "其他文本"}</td><td>{row.tracked ? "已跟踪" : "未跟踪 / 忽略"}</td><td>{row.lines}</td><td>{row.hotspotScore.toFixed(1)}</td></tr>{/each}</tbody></table></section>
	{:else if report === "authors" && authors}
		<section class="panel"><table><thead><tr><th>作者</th><th>邮箱</th><th>提交</th><th>增删</th><th>当前所有权</th></tr></thead><tbody>{#each authors.items as row}<tr><td>{row.name}</td><td>{row.email}</td><td>{row.commitCount}</td><td>+{row.additions} / -{row.deletions}</td><td>{row.ownedLines}</td></tr>{/each}</tbody></table></section>
	{:else if report === "languages"}
		<section class="panel"><table><thead><tr><th>语言</th><th>文件</th><th>代码行</th><th>占比</th></tr></thead><tbody>{#each languages as row}<tr><td>{row.language}</td><td>{row.files}</td><td>{row.lines}</td><td>{row.percentage.toFixed(1)}%</td></tr>{/each}</tbody></table></section>
	{:else if report === "coupling" && couplings}
		<section class="panel"><table><thead><tr><th>文件 A</th><th>文件 B</th><th>共变次数</th><th>强度</th></tr></thead><tbody>{#each couplings.items as row}<tr><td>{row.left}</td><td>{row.right}</td><td>{row.coChanges}</td><td>{row.strength.toFixed(1)}%</td></tr>{/each}</tbody></table></section>
	{:else if report === "ownership" && ownership}
		<section class="panel"><table><thead><tr><th>文件</th><th>作者</th><th>行数</th><th>占比</th><th>平均年龄</th></tr></thead><tbody>{#each ownership.items as row}<tr><td>{row.path}</td><td>{row.authorName}</td><td>{row.lines}</td><td>{row.percentage.toFixed(1)}%</td><td>{row.ageDays} 天</td></tr>{/each}</tbody></table></section>
	{:else if report === "directory-ownership" && directoryOwnership}
		<section class="panel"><table><thead><tr><th>目录</th><th>主负责人</th><th>行数</th><th>文件数</th></tr></thead><tbody>{#each directoryOwnership.items as row}<tr><td>{row.directory}</td><td>{row.dominantAuthor ?? "未知"}</td><td>{row.lines}</td><td>{row.files}</td></tr>{/each}</tbody></table></section>
	{:else if report === "code-age"}
		<section class="panel"><table><thead><tr><th>年龄桶</th><th>文件</th><th>代码行</th><th>占比</th></tr></thead><tbody>{#each codeAge as row}<tr><td>{row.label}</td><td>{row.files}</td><td>{row.lines}</td><td>{row.percentage.toFixed(1)}%</td></tr>{/each}</tbody></table></section>
	{:else if report === "bus-factor" && busFactor}
		<div class="cards"><div><span>LOC 口径</span><b>{busFactor.loc}</b><small>覆盖 50% 当前代码所需作者数</small></div><div><span>提交口径</span><b>{busFactor.commits}</b><small>覆盖 50% 历史提交所需作者数</small></div></div>
	{:else if isEvidenceReport(report) && evidence}
		<section class="panel"><div class="evidence-meta"><span>{evidence.available ? "已完成" : "不可用"}</span><span>{evidence.items.length} 条证据</span></div><p>{evidence.conclusion ?? evidence.limitations[0] ?? "暂无结论"}</p><table><thead><tr><th>类型</th><th>值</th><th>文件</th><th>行号</th><th>说明</th></tr></thead><tbody>{#each evidence.items as row}<tr><td>{row.kind}</td><td>{row.value}</td><td>{#if row.path}<a href={filePreviewHref(projectId, row.path, row.line)}>{row.path}</a>{:else}—{/if}</td><td>{row.line ?? "—"}</td><td>{row.conclusion}</td></tr>{/each}</tbody></table></section>
	{:else if report === "refs" && refs}
		<section class="panel"><table><thead><tr><th>Ref</th><th>目标</th><th>状态</th></tr></thead><tbody>{#each refs.items as row}<tr><td>{row.name}</td><td><code>{row.target ?? "—"}</code></td><td>{row.excluded ? "GitButler 内部" : "可见"}</td></tr>{/each}</tbody></table></section>
	{:else if report === "diagnostics" && diagnostics}
		<section class="panel diagnostics"><div><span>仓库类型</span><b>{diagnostics.bare ? "裸仓库" : "普通仓库"}</b></div><div><span>可见 refs</span><b>{diagnostics.refs}</b></div><div><span>Stash</span><b>{diagnostics.stash ? "存在" : "无"}</b></div><div><span>HEAD Reflog</span><b>{diagnostics.reflog ? "存在" : "无"}</b></div><div><span>链接 Worktree</span><b>{diagnostics.worktrees}</b></div><div><span>Submodule</span><b>{diagnostics.submodules ? "存在" : "无"}</b></div><div><span>Hooks</span><b>{diagnostics.hooks} 个（不执行）</b></div><div><span>LFS</span><b>{diagnostics.lfs ? "存在" : "无"}</b></div><div><span>浅克隆</span><b>{diagnostics.shallow ? "是" : "否"}</b></div></section>
	{:else}
		<div class="empty">暂无数据。请先在总览页完成一次分析。</div>
	{/if}

	{#if isPagedReport(report) && pageTotal > 0}
		<nav class="pagination" aria-label="分页">
			<button onclick={() => changePage(-1)} disabled={pageNumber <= 1}>上一页</button>
			<span>第 {pageNumber} / {pageCount} 页 · 共 {pageTotal.toLocaleString()} 条</span>
			<button onclick={() => changePage(1)} disabled={pageNumber >= pageCount}>下一页</button>
		</nav>
	{/if}
</div>

<style>
	.report-shell { width: 100%; height: 100%; overflow: auto; padding: 28px; color: var(--text-1); background: var(--bg-2); }
	.report-header { display: flex; justify-content: space-between; align-items: flex-start; gap: 16px; margin-bottom: 18px; }
	.back { color: var(--text-3); font-size: 11px; text-decoration: none; }
	h1 { margin: 5px 0 0; font-size: 23px; } p { margin: 6px 0 0; color: var(--text-3); font-size: 12px; }
	.status { padding: 4px 7px; border-radius: 5px; background: var(--bg-1); color: var(--text-3); font-size: 11px; }
	.panel { padding: 16px; border: 1px solid var(--border-1); border-radius: 8px; background: var(--bg-1); }
	.cards { display: grid; grid-template-columns: repeat(4, minmax(130px, 1fr)); gap: 10px; }
	.cards > div { display: flex; flex-direction: column; gap: 6px; padding: 16px; border: 1px solid var(--border-1); border-radius: 8px; background: var(--bg-1); }
	.cards span, .cards small, .count, .evidence-meta { color: var(--text-3); font-size: 11px; } .cards b { font-size: 22px; } .cards small { line-height: 1.4; }
	table { width: 100%; border-collapse: collapse; font-size: 11px; } th { padding: 8px 5px; color: var(--text-3); font-weight: 500; text-align: left; } td { max-width: 360px; padding: 8px 5px; overflow: hidden; border-top: 1px solid var(--border-1); text-overflow: ellipsis; white-space: nowrap; } code { color: #c8a96b; } .positive { color: #9bcf8a; } .negative { color: #df9898; } .delta { color: var(--text-2); }
	.count { margin-bottom: 8px; } .evidence-meta { display: flex; justify-content: space-between; margin-bottom: 6px; } .error { margin-bottom: 16px; padding: 10px 12px; border-radius: 6px; background: rgba(192, 83, 83, .16); color: #e99a9a; font-size: 12px; } .empty { padding: 72px 10px; color: var(--text-3); text-align: center; font-size: 12px; }
	.diagnostics { display: grid; grid-template-columns: repeat(3, minmax(150px, 1fr)); gap: 12px; } .diagnostics div { display: flex; justify-content: space-between; gap: 8px; } .diagnostics span { color: var(--text-3); } .diagnostics b { font-weight: 500; }
	.pagination { display: flex; align-items: center; justify-content: center; gap: 12px; margin-top: 14px; color: var(--text-3); font-size: 11px; } .pagination button { padding: 6px 10px; border: 1px solid var(--border-1); border-radius: 5px; background: var(--bg-1); color: var(--text-1); cursor: pointer; font-size: 11px; } .pagination button:disabled { opacity: .45; cursor: default; }
	@media (max-width: 760px) { .report-shell { padding: 16px; } .cards { grid-template-columns: repeat(2, 1fr); } .panel { overflow-x: auto; } table { min-width: 620px; } }
</style>
