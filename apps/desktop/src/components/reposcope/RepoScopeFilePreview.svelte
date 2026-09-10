<script lang="ts">
	import { onMount } from "svelte";
	import { page } from "$app/state";
	import RepoScopeNavigation from "$components/reposcope/RepoScopeNavigation.svelte";
	import { RepoScopeService } from "$lib/reposcope/service.svelte";
	import type { FileContent } from "$lib/reposcope/types";

	const projectId = $derived(page.params.projectId!);
	const sourcePath = $derived(page.url.searchParams.get("path") ?? "");
	const sourceLine = $derived(Math.max(1, Number(page.url.searchParams.get("line") ?? "1") || 1));
	const revision = $derived(page.url.searchParams.get("revision") ?? undefined);
	const service = new RepoScopeService();
	let content = $state<FileContent>();
	let error = $state<string>();

	onMount(async () => {
		if (!sourcePath) {
			error = "缺少文件路径";
			return;
		}
		try {
			content = await service.fileContent(projectId, sourcePath, sourceLine, revision);
		} catch (cause) {
			error = cause instanceof Error ? cause.message : "读取源码失败";
		}
	});
</script>

<div class="preview-shell">
	<RepoScopeNavigation />
	<a class="back" href={`/${projectId}/insights/files`}>← 返回文件演化</a>
	{#if error}<div class="error">{error}</div>{/if}
	{#if content}
		<header>
			<div><div class="eyebrow">{content.language ?? "源码"}</div><h1>{content.path}</h1></div>
			{#if content.revision}<code>{content.revision.slice(0, 12)}</code>{/if}
		</header>
		{#if content.truncated}<div class="notice">文件超过 150 万字符，已截断显示。</div>{/if}
		<div class="source" aria-label="源码预览">
			{#each content.content.split("\n") as line, index}
				<div class:highlight={index + 1 === content.highlightLine} class="source-line"><span class="line-number">{index + 1}</span><code>{line || " "}</code></div>
			{/each}
		</div>
	{:else if !error}
		<div class="empty">正在读取源码…</div>
	{/if}
</div>

<style>
	.preview-shell { width: 100%; height: 100%; overflow: auto; padding: 28px; color: var(--text-1); background: var(--bg-2); }
	.back { color: var(--text-3); font-size: 11px; text-decoration: none; }
	header { display: flex; justify-content: space-between; align-items: flex-start; gap: 16px; margin: 12px 0 16px; } h1 { margin: 4px 0 0; font-size: 20px; overflow-wrap: anywhere; } .eyebrow { color: var(--text-3); font-size: 11px; } header code { color: #c8a96b; font-size: 11px; }
	.notice, .error { margin-bottom: 12px; padding: 9px 12px; border-radius: 6px; font-size: 12px; } .notice { color: var(--text-3); background: var(--bg-1); } .error { color: #e99a9a; background: rgba(192, 83, 83, .16); }
	.source { overflow: auto; padding: 10px 0; border: 1px solid var(--border-1); border-radius: 8px; background: var(--bg-1); font: 12px/1.55 ui-monospace, SFMono-Regular, Consolas, monospace; }
	.source-line { display: grid; grid-template-columns: 58px minmax(max-content, 1fr); min-width: max-content; padding-right: 18px; white-space: pre; } .source-line.highlight { background: rgba(169, 209, 142, .16); } .line-number { padding-right: 12px; color: var(--text-3); text-align: right; user-select: none; } .source-line code { color: var(--text-1); }
	.empty { padding: 72px 10px; color: var(--text-3); text-align: center; font-size: 12px; }
</style>
