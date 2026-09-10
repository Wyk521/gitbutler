<script lang="ts">
	import { GIT_CONFIG_SERVICE } from "$lib/config/gitConfigService";
	import { inject } from "@gitbutler/core/context";
	import { CardGroup, Toggle } from "@gitbutler/ui";
	import { onMount } from "svelte";

	const gitConfig = inject(GIT_CONFIG_SERVICE);
	let annotateCommits = $state(true);

	function toggleCommitterSigning() {
		annotateCommits = !annotateCommits;
		gitConfig.set("gitbutler.gitbutlerCommitter", annotateCommits ? "1" : "0");
	}

	onMount(async () => {
		annotateCommits = (await gitConfig.get("gitbutler.gitbutlerCommitter")) === "1";
	});
</script>

<CardGroup.Item standalone labelFor="committerSigning">
	{#snippet title()}保留提交来源标记{/snippet}
	{#snippet caption()}
		新建的 GitButler 工作区提交可以保留来源标记。该设置只写入本地 Git 配置，不会联网。
	{/snippet}
	{#snippet actions()}
		<Toggle id="committerSigning" checked={annotateCommits} onclick={toggleCommitterSigning} />
	{/snippet}
</CardGroup.Item>

<CardGroup.Item standalone>
	{#snippet title()}自动获取远程更新{/snippet}
	{#snippet caption()}严格离线版本已禁用 fetch、push、clone 和自动同步。{/snippet}
	{#snippet actions()}<span class="disabled-pill">已禁用</span>{/snippet}
</CardGroup.Item>

<style>
	.disabled-pill {
		padding: 4px 8px;
		border-radius: 999px;
		background: var(--bg-2);
		color: var(--text-3);
		font-size: 12px;
	}
</style>
