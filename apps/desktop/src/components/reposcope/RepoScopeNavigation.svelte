<script lang="ts">
	import { page } from "$app/state";

	const projectId = $derived(page.params.projectId!);
	const groups = [
		{
			label: "项目画像",
			items: [
				["overview", "总览"],
				["activity", "活跃度"],
			],
		},
		{
			label: "代码演化",
			items: [
				["commits", "提交"],
				["files", "文件"],
				["authors", "贡献者"],
				["languages", "语言"],
				["hotspots", "热点"],
				["coupling", "变更耦合"],
			],
		},
		{
			label: "维护风险",
			items: [
				["ownership", "代码所有权"],
				["directory-ownership", "目录所有权"],
				["code-age", "代码年龄"],
				["bus-factor", "Bus Factor"],
			],
		},
		{
			label: "审计证据",
			items: [
				["delivery", "交付溯源"],
				["footprints", "外部系统"],
				["regions", "地域线索"],
				["dependencies", "依赖与插件"],
			],
		},
		{
			label: "仓库诊断",
			items: [
				["diagnostics", "工作区状态"],
				["refs", "Refs / 标签"],
			],
		},
	] as const;

	function active(key: string) {
		return page.url.pathname === `/${projectId}/insights/${key}`;
	}
</script>

<nav class="insights-nav" aria-label="仓库洞察导航">
	{#each groups as group}
		<div class="group">
			<div class="group-title">{group.label}</div>
			<div class="links">
				{#each group.items as [key, label]}
					<a class:active={active(key)} href={`/${projectId}/insights/${key}`}>{label}</a>
				{/each}
			</div>
		</div>
	{/each}
</nav>

<style>
	.insights-nav { display: flex; flex-wrap: wrap; gap: 14px; margin-bottom: 18px; padding: 10px 12px; border: 1px solid var(--border-1); border-radius: 8px; background: var(--bg-1); }
	.group { min-width: 126px; }
	.group-title { margin-bottom: 6px; color: var(--text-3); font-size: 10px; }
	.links { display: flex; flex-wrap: wrap; gap: 4px; }
	a { padding: 4px 7px; border-radius: 5px; color: var(--text-2); font-size: 11px; text-decoration: none; }
	a:hover, a.active { background: var(--bg-3); color: var(--text-1); }
	a.active { box-shadow: inset 0 -1px #a9d18e; }
</style>
