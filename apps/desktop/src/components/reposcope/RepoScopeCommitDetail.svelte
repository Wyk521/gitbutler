<script lang="ts">
	import { onMount } from "svelte";
	import RepoScopeNavigation from "$components/reposcope/RepoScopeNavigation.svelte";
	import { filePreviewHref, RepoScopeService } from "$lib/reposcope/service.svelte";
	import type { CommitDetail } from "$lib/reposcope/types";

	const { projectId, oid }: { projectId: string; oid: string } = $props();
	const service = new RepoScopeService();
	let detail = $state<CommitDetail>();
	let error = $state<string>();

	onMount(async () => {
		try {
			detail = (await service.commitDetail(projectId, oid)) ?? undefined;
			if (!detail) error = "找不到该提交";
		} catch (cause) {
			error = cause instanceof Error ? cause.message : "读取提交详情失败";
		}
	});
</script>

<div class="detail-shell">
	<RepoScopeNavigation />
	<a class="back" href={`/${projectId}/insights/commits`}>← 返回提交</a>
	{#if error}<div class="error">{error}</div>{/if}
	{#if detail}
		<header><code>{detail.commit.oid}</code><h1>{detail.commit.subject || "（无标题）"}</h1><p>{detail.commit.authorName} · {new Date(detail.commit.authoredAt).toLocaleString("zh-CN")} · {detail.commit.category}</p></header>
		<section class="panel"><h2>文件变更（{detail.files.length}）</h2><table><thead><tr><th>文件</th><th>旧路径</th><th>新增</th><th>删除</th></tr></thead><tbody>{#each detail.files as file}<tr><td><a href={filePreviewHref(projectId, file.path, undefined, detail.commit.oid)}>{file.path}</a></td><td>{file.previousPath ?? "—"}</td><td class="positive">+{file.additions}</td><td class="negative">-{file.deletions}</td></tr>{/each}</tbody></table></section>
	{/if}
</div>

<style>
	.detail-shell { width: 100%; height: 100%; overflow: auto; padding: 28px; color: var(--text-1); background: var(--bg-2); }
	.back { color: var(--text-3); font-size: 11px; text-decoration: none; } header { margin: 10px 0 18px; } h1 { margin: 6px 0; font-size: 22px; } h2 { margin: 0 0 12px; font-size: 14px; } p { margin: 0; color: var(--text-3); font-size: 12px; } code { color: #c8a96b; font-size: 11px; }
	.panel { padding: 16px; border: 1px solid var(--border-1); border-radius: 8px; background: var(--bg-1); } table { width: 100%; border-collapse: collapse; font-size: 11px; } th, td { padding: 8px 5px; text-align: left; } th { color: var(--text-3); font-weight: 500; } td { border-top: 1px solid var(--border-1); } .positive { color: #9bcf8a; } .negative { color: #df9898; } .error { margin: 16px 0; padding: 10px; color: #e99a9a; background: rgba(192, 83, 83, .16); }
</style>
