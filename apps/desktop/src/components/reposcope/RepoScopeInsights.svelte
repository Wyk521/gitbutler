<script lang="ts">
	import { onMount } from "svelte";
	import { RepoScopeService } from "$lib/reposcope/service.svelte";
	import RepoScopeNavigation from "$components/reposcope/RepoScopeNavigation.svelte";
	import type {
		AnalysisStatus,
		CommitSummary,
		DailyActivity,
		FileSummary,
		LanguageSummary,
		Overview,
		Page,
		RepositoryDiagnostics,
		ReportSnapshot,
	} from "$lib/reposcope/types";

	const { projectId }: { projectId: string } = $props();
	const service = new RepoScopeService();

	let status = $state<AnalysisStatus>();
	let overview = $state<Overview>();
	let activity = $state<DailyActivity[]>([]);
	let commits = $state<Page<CommitSummary>>();
	let files = $state<Page<FileSummary>>();
	let languages = $state<LanguageSummary[]>([]);
	let diagnostics = $state<RepositoryDiagnostics>();
	let reports = $state<ReportSnapshot[]>([]);
	let loading = $state(false);
	let error = $state<string>();
	let stopAnalysisEvents: (() => void) | undefined;
	let stopFreshness: (() => void) | undefined;

	const statusLabel: Record<AnalysisStatus["status"], string> = {
		queued: "排队中",
		running: "分析中",
		complete: "最新",
		partial: "部分完成",
		failed: "失败",
		cancelled: "已取消",
		interrupted: "已中断",
	};

	const phaseLabel: Record<AnalysisStatus["phase"], string> = {
		queued: "等待任务",
		history: "读取提交历史",
		worktree: "扫描工作目录",
		metrics: "计算指标",
		ownership: "计算所有权",
		evidence: "扫描审计线索",
		persisting: "保存分析结果",
	};

	function reportTitle(kind: string) {
		return kind === "delivery" ? "交付溯源" : kind === "footprints" ? "外部系统" : kind === "regions" ? "地域线索" : "依赖与插件";
	}

	async function loadReports() {
		const [nextOverview, nextActivity, nextCommits, nextFiles, nextLanguages, nextDiagnostics, delivery, footprints, regions, dependencies] =
			await Promise.all([
				service.overview(projectId),
				service.activity(projectId),
				service.commits(projectId),
				service.files(projectId),
				service.languages(projectId),
				service.diagnostics(projectId),
				service.report(projectId, "delivery"),
				service.report(projectId, "footprints"),
				service.report(projectId, "regions"),
				service.report(projectId, "dependencies"),
			]);
		overview = nextOverview ?? undefined;
		activity = nextActivity;
		commits = nextCommits;
		files = nextFiles;
		languages = nextLanguages;
		diagnostics = nextDiagnostics;
		reports = [delivery, footprints, regions, dependencies];
	}

	async function refreshStatus() {
		try {
			status = (await service.status(projectId)) ?? undefined;
			// The latest run can be failed/cancelled/interrupted while an older
			// successful batch remains active.  Always load the active snapshots so
			// those results stay visible alongside the retry error.
			if (status) await loadReports();
		} catch (cause) {
			error = cause instanceof Error ? cause.message : "无法读取分析状态";
		}
	}

	async function startAnalysis() {
		loading = true;
		error = undefined;
		try {
			status = await service.start(projectId);
			await refreshStatus();
		} catch (cause) {
			error = cause instanceof Error ? cause.message : "启动分析失败";
		} finally {
			loading = false;
		}
	}

	async function cancelAnalysis() {
		try {
			status = await service.cancel(projectId);
		} catch (cause) {
			error = cause instanceof Error ? cause.message : "取消分析失败";
		}
	}

	onMount(() => {
		void refreshStatus();
		const unlisten = service.listen(projectId, (nextStatus) => {
			status = nextStatus;
			if (nextStatus.status === "complete" || nextStatus.status === "partial") void refreshStatus();
		});
		// The event is authoritative; the frontend does not poll while a scan is
		// running.  This keeps a large repository's UI independent from the
		// analysis worker's cadence and avoids duplicate SQLite reads.
		stopAnalysisEvents = () => {
			void unlisten.then((dispose) => dispose());
		};
		const freshnessListeners = service.listenRepositoryChanges(projectId, () => void refreshStatus());
		stopFreshness = () => {
			void freshnessListeners.then((dispose) => dispose());
		};
		return () => {
			stopAnalysisEvents?.();
			stopFreshness?.();
		};
	});
</script>

<div class="insights" aria-label="仓库洞察">
	<RepoScopeNavigation />
	<header class="insights__header">
		<div>
			<div class="eyebrow">RepoScope Desktop</div>
			<h1>仓库洞察</h1>
			<p>本地离线分析 · 结论仅作为线索和辅助判断</p>
		</div>
		<div class="header-actions">
			{#if status && (status.status === "running" || status.status === "queued")}
				<button class="secondary" onclick={cancelAnalysis}>取消分析</button>
			{:else}
				<button class="primary" onclick={startAnalysis} disabled={loading}>
					{loading ? "启动中…" : "重新分析"}
				</button>
			{/if}
		</div>
	</header>

	{#if status}
		<div class:running={status.status === "running"} class="status-strip">
			<span class="status-dot"></span>
			<strong>{statusLabel[status.status]}</strong>
			{#if status.status === "running"}
				<span>{phaseLabel[status.phase]}</span>
				<span>{Math.round(status.progress * 100)}%</span>
				{#if status.total > 0}<span>{status.processed} / {status.total}</span>{/if}
			{:else if status.freshness && status.freshness !== "fresh"}
				<span>结果已过期：{status.freshness}</span>
			{/if}
			{#if status.status === "running"}<progress max="1" value={status.progress}></progress>{/if}
		</div>
	{/if}

	{#if error || status?.error}
		<div class="error">{error ?? status?.error}</div>
	{/if}

	{#if overview}
		<section class="cards">
			<div class="card"><span>提交</span><strong>{overview.commitCount.toLocaleString()}</strong></div>
			<div class="card"><span>当前文件</span><strong>{overview.fileCount.toLocaleString()}</strong></div>
			<div class="card"><span>代码行</span><strong>{overview.lines.toLocaleString()}</strong></div>
			<div class="card"><span>贡献者</span><strong>{overview.authorCount.toLocaleString()}</strong></div>
			<div class="card"><span>变更量</span><strong>{overview.churn.toLocaleString()}</strong><small>净增长 {overview.netGrowth >= 0 ? "+" : ""}{overview.netGrowth.toLocaleString()}</small></div>
			<div class="card"><span>分支 / 标签</span><strong>{overview.branchCount} / {overview.tagCount}</strong><small>未跟踪文件 {overview.untrackedFiles}</small></div>
		</section>

		<div class="columns">
			<section class="panel">
				<div class="panel__title"><h2>近期活跃度</h2><span>UTC</span></div>
				{#if activity.length}
					<div class="activity-list">
						{#each activity.slice(-14).reverse() as day}
							<div class="activity-row"><span>{day.date}</span><span>{day.commits} 次提交</span><span class="positive">+{day.additions}</span><span class="negative">-{day.deletions}</span></div>
						{/each}
					</div>
				{:else}<div class="empty">暂无提交活动</div>{/if}
			</section>
			<section class="panel">
				<div class="panel__title"><h2>语言分布</h2><span>当前文件</span></div>
				{#if languages.length}
					{#each languages.slice(0, 8) as language}
						<div class="language-row"><span>{language.language}</span><div class="bar"><i style={`width: ${Math.min(100, language.percentage)}%`}></i></div><b>{language.percentage.toFixed(1)}%</b></div>
					{/each}
				{:else}<div class="empty">暂无语言统计</div>{/if}
			</section>
		</div>

		<div class="columns">
			<section class="panel wide">
				<div class="panel__title"><h2>热点文件</h2><span>变更量 / 频率 / 当前规模</span></div>
				{#if files?.items.length}
					<table><thead><tr><th>文件</th><th>语言</th><th>热点</th><th>提交</th><th>增删</th></tr></thead><tbody>
						{#each files.items as file}<tr><td title={file.path}>{file.path}</td><td>{file.language ?? "未知"}</td><td><b>{file.hotspotScore.toFixed(1)}</b></td><td>{file.commitCount}</td><td><span class="positive">+{file.additions}</span> <span class="negative">-{file.deletions}</span></td></tr>{/each}
					</tbody></table>
				{:else}<div class="empty">完成一次分析后显示热点文件</div>{/if}
			</section>
			<section class="panel">
				<div class="panel__title"><h2>仓库诊断</h2><span>只读</span></div>
				{#if diagnostics}
					<div class="diagnostics"><div>工作目录 <b>{diagnostics.bare ? "裸仓库" : "可用"}</b></div><div>Refs <b>{diagnostics.refs}</b></div><div>Stash <b>{diagnostics.stash ? "有" : "无"}</b></div><div>Reflog <b>{diagnostics.reflog ? "有" : "无"}</b></div><div>Worktree <b>{diagnostics.worktrees}</b></div><div>Hooks <b>{diagnostics.hooks} 个（不执行）</b></div><div>LFS <b>{diagnostics.lfs ? "有" : "无"}</b></div><div>浅克隆 <b>{diagnostics.shallow ? "是" : "否"}</b></div></div>
				{:else}<div class="empty">暂无诊断信息</div>{/if}
			</section>
		</div>

		<section class="panel commits-panel">
			<div class="panel__title"><h2>最近提交</h2><span>{commits?.total ?? 0} 条</span></div>
			{#if commits?.items.length}
				<div class="commit-list">{#each commits.items as commit}<div class="commit-row"><code>{commit.shortOid}</code><span class="commit-subject">{commit.subject || "（无标题）"}</span><span class="badge">{commit.category}</span><span>{commit.authorName}</span><time>{new Date(commit.authoredAt).toLocaleDateString("zh-CN")}</time></div>{/each}</div>
			{:else}<div class="empty">暂无提交</div>{/if}
		</section>

		<section class="panel evidence-panel">
			<div class="panel__title"><h2>审计证据入口</h2><span>本地只读线索</span></div>
			<div class="evidence-grid">
				{#each reports as report}
					<div class="evidence-card">
						<div><strong>{reportTitle(report.kind)}</strong><span>{report.items.length} 条</span></div>
						<p>{report.conclusion ?? report.limitations[0] ?? "已完成扫描"}</p>
						{#if report.items.length}<small>{report.items[0].value}</small>{/if}
					</div>
				{/each}
			</div>
		</section>
	{:else}
		<div class="empty-state"><div class="empty-state__icon">⌁</div><h2>还没有分析结果</h2><p>RepoScope 会读取本机仓库的提交、文件和工作目录，不会访问网络。</p><button class="primary" onclick={startAnalysis} disabled={loading}>{loading ? "启动中…" : "开始首次分析"}</button></div>
	{/if}
</div>

<style>
	.insights { width: 100%; height: 100%; overflow: auto; padding: 28px; color: var(--text-1); background: var(--bg-2); }
	.insights__header { display: flex; justify-content: space-between; gap: 16px; align-items: flex-start; margin-bottom: 20px; }
	h1 { margin: 4px 0; font-size: 24px; } h2 { margin: 0; font-size: 14px; } p { margin: 6px 0 0; color: var(--text-3); font-size: 12px; }
	.eyebrow { color: var(--text-3); font-size: 11px; letter-spacing: .08em; text-transform: uppercase; }
	.header-actions button, .empty-state button { border: 0; border-radius: 6px; padding: 9px 14px; cursor: pointer; font-size: 12px; }
	.primary { color: #172018; background: #a9d18e; } .secondary { color: var(--text-1); background: var(--bg-1); border: 1px solid var(--border-1) !important; }
	button:disabled { opacity: .55; cursor: default; }
	.status-strip { display: flex; align-items: center; gap: 10px; min-height: 38px; margin-bottom: 16px; padding: 0 12px; border: 1px solid var(--border-1); border-radius: 7px; background: var(--bg-1); color: var(--text-3); font-size: 12px; }
	.status-strip strong { color: var(--text-1); } .status-dot { width: 7px; height: 7px; border-radius: 50%; background: #7a8491; } .status-strip.running .status-dot { background: #a9d18e; } progress { width: 140px; height: 6px; margin-left: auto; accent-color: #a9d18e; }
	.error { margin-bottom: 16px; padding: 10px 12px; border-radius: 6px; background: rgba(192, 83, 83, .16); color: #e99a9a; font-size: 12px; }
	.cards { display: grid; grid-template-columns: repeat(5, minmax(110px, 1fr)); gap: 10px; margin-bottom: 14px; } .card, .panel { border: 1px solid var(--border-1); border-radius: 8px; background: var(--bg-1); } .card { display: flex; flex-direction: column; gap: 6px; padding: 14px; } .card span, .card small, .panel__title span { color: var(--text-3); font-size: 11px; } .card strong { font-size: 22px; } .card small { font-size: 10px; }
	.columns { display: grid; grid-template-columns: minmax(0, 1.45fr) minmax(280px, 1fr); gap: 14px; margin-bottom: 14px; } .panel { padding: 16px; min-width: 0; } .panel.wide { min-height: 280px; } .panel__title { display: flex; justify-content: space-between; align-items: baseline; margin-bottom: 12px; }
	.activity-list { display: grid; gap: 6px; } .activity-row { display: grid; grid-template-columns: 1fr auto auto auto; gap: 10px; font-size: 11px; color: var(--text-3); } .positive { color: #9bcf8a; } .negative { color: #df9898; }
	.language-row { display: grid; grid-template-columns: 90px 1fr 48px; align-items: center; gap: 8px; margin: 9px 0; font-size: 11px; } .bar { height: 6px; overflow: hidden; border-radius: 4px; background: var(--bg-3); } .bar i { display: block; height: 100%; border-radius: inherit; background: #a9d18e; } .language-row b { text-align: right; font-weight: 500; color: var(--text-3); }
	table { width: 100%; border-collapse: collapse; font-size: 11px; } th { padding: 7px 4px; text-align: left; color: var(--text-3); font-weight: 500; } td { max-width: 280px; padding: 8px 4px; overflow: hidden; border-top: 1px solid var(--border-1); text-overflow: ellipsis; white-space: nowrap; } td:first-child { color: var(--text-1); }
	.diagnostics { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; font-size: 11px; color: var(--text-3); } .diagnostics div { display: flex; justify-content: space-between; gap: 8px; } .diagnostics b { color: var(--text-1); font-weight: 500; text-align: right; }
	.commits-panel { margin-bottom: 24px; } .commit-list { display: grid; } .commit-row { display: grid; grid-template-columns: 58px minmax(100px, 1fr) 56px 100px 84px; align-items: center; gap: 8px; padding: 9px 0; border-top: 1px solid var(--border-1); color: var(--text-3); font-size: 11px; } .commit-row code { color: #c8a96b; } .commit-subject { overflow: hidden; color: var(--text-1); text-overflow: ellipsis; white-space: nowrap; } .badge { width: max-content; padding: 2px 5px; border-radius: 4px; background: var(--bg-3); font-size: 9px; } time { text-align: right; }
	.evidence-panel { margin-bottom: 24px; } .evidence-grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 8px; } .evidence-card { min-width: 0; padding: 10px; border: 1px solid var(--border-1); border-radius: 6px; background: var(--bg-2); } .evidence-card > div { display: flex; justify-content: space-between; gap: 6px; font-size: 11px; } .evidence-card > div span, .evidence-card p, .evidence-card small { color: var(--text-3); font-size: 10px; } .evidence-card p { min-height: 28px; margin: 8px 0; } .evidence-card small { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
	.empty, .empty-state { color: var(--text-3); font-size: 12px; } .empty { padding: 18px 0 4px; text-align: center; } .empty-state { display: grid; place-items: center; align-content: center; min-height: 420px; text-align: center; } .empty-state__icon { margin-bottom: 10px; font-size: 34px; color: #a9d18e; } .empty-state h2 { margin-bottom: 2px; font-size: 16px; color: var(--text-1); } .empty-state button { margin-top: 14px; }
	@media (max-width: 900px) { .cards { grid-template-columns: repeat(3, 1fr); } .columns { grid-template-columns: 1fr; } .evidence-grid { grid-template-columns: repeat(2, 1fr); } } @media (max-width: 600px) { .insights { padding: 16px; } .cards { grid-template-columns: repeat(2, 1fr); } .commit-row { grid-template-columns: 52px minmax(80px, 1fr) 48px; } .commit-row span:nth-of-type(2), .commit-row time { display: none; } .evidence-grid { grid-template-columns: 1fr; } }
</style>
