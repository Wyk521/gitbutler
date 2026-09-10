<script lang="ts">
	import { goto } from "$app/navigation";
	import SettingsSection from "$components/shared/SettingsSection.svelte";
	import { BACKEND } from "$lib/backend";
	import { inject } from "@gitbutler/core/context";
	import { persisted } from "@gitbutler/shared/persisted";
	import { Button, InfoMessage, type MessageStyle, Spacer, Textbox } from "@gitbutler/ui";

	import { onMount } from "svelte";

	const backend = inject(BACKEND);

	let loading = $state(false);
	let errors = $state<{ label: string }[]>([]);
	let completed = $state(false);
	let repositoryUrl = $state("");
	let targetDirPath = $state("");
	let savedTargetDirPath = persisted("", "clone_targetDirPath");

	onMount(async () => {
		if ($savedTargetDirPath) {
			targetDirPath = $savedTargetDirPath;
		} else {
			targetDirPath = await backend.documentDir();
		}
	});

	async function handleCloneTargetSelect() {
		const selectedPath = await backend.filePicker({
			directory: true,
			recursive: true,
			title: "Target Clone Directory",
		});
		if (!selectedPath || !selectedPath[0]) return;

		targetDirPath = Array.isArray(selectedPath) ? selectedPath[0] : selectedPath;
	}

	function cloneRepository() {
		loading = true;
		savedTargetDirPath.set(targetDirPath);
		errors = [{ label: "离线版本不支持远程克隆，请先在本机准备仓库后再添加。" }];
		loading = false;
	}

	function handleCancel() {
		if (history.length > 0) {
			history.back();
		} else {
			goto("/");
		}
	}
</script>

<h1 class="clone-title text-serif-42">远程克隆已关闭</h1>
<SettingsSection>
	<Textbox label="远程仓库地址（已关闭）" bind:value={repositoryUrl} disabled />

	<div class="clone__field repositoryTargetPath">
		<Textbox
			label="克隆目录（仅作提示）"
			bind:value={targetDirPath}
			placeholder="选择本机目录"
		/>
		<Button kind="outline" disabled={loading} onclick={handleCloneTargetSelect}>选择目录</Button>
	</div>
</SettingsSection>

<Spacer dotted margin={24} />

{#if completed}
	{@render Notification({ title: "完成", style: "success" })}
{/if}
{#if errors.length}
	{@render Notification({ title: "不可用", items: errors, style: "danger" })}
{/if}

<div class="clone__actions">
	<Button kind="outline" disabled={loading} onclick={handleCancel}>返回</Button>
	<Button
		style="pop"
		icon={errors.length > 0 ? "refresh" : "chevron-right"}
		disabled={loading}
		{loading}
		onclick={cloneRepository}
	>
			{#if loading}
			处理中…
		{:else if errors.length > 0}
			重试提示
		{:else}
			远程克隆
		{/if}
	</Button>
</div>

{#snippet Notification({
	title: titleLabel,
	items,
	style,
}: {
	title: string;
	items?: any[];
	style: MessageStyle;
})}
	<div class="clone__info-message">
		<InfoMessage {style} filled outlined={false}>
			{#snippet title()}
				{titleLabel}
			{/snippet}
			{#snippet content()}
				{#if items && items.length > 0}
					{#each items as item}
						<span>{item.label}</span>
					{/each}
				{/if}
			{/snippet}
		</InfoMessage>
	</div>
{/snippet}

<style>
	.clone-title {
		margin-bottom: 20px;
		color: var(--text-1);
		line-height: 1;
	}

	.clone__field {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.clone__actions {
		display: flex;
		justify-content: end;
		gap: 8px;
	}

	.clone__info-message {
		margin-bottom: 20px;
	}
</style>
