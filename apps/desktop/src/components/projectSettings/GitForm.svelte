<script lang="ts">
	import CommitSigningForm from "$components/projectSettings/CommitSigningForm.svelte";
	import GitHooksForm from "$components/projectSettings/GitHooksForm.svelte";
	import KeysForm from "$components/projectSettings/KeysForm.svelte";
	import SettingsSection from "$components/shared/SettingsSection.svelte";
	import { BACKEND } from "$lib/backend";
	import { inject } from "@gitbutler/core/context";
	import { CardGroup, Spacer } from "@gitbutler/ui";

	const { projectId }: { projectId: string } = $props();
	const backend = inject(BACKEND);
</script>

<SettingsSection>
	<CardGroup>
		<CardGroup.Item>
			{#snippet title()}远程发布与直接合并{/snippet}
			{#snippet caption()}严格离线版本不执行 fetch、push、PR 或远程分支删除；本地工作区提交和分支编辑仍可使用。{/snippet}
			{#snippet actions()}<span class="disabled-pill">已禁用</span>{/snippet}
		</CardGroup.Item>
	</CardGroup>

	<GitHooksForm {projectId} />
	<CommitSigningForm {projectId} />
	{#if backend.platformName !== "windows"}
		<Spacer />
		<KeysForm {projectId} showProjectName={false} />
	{/if}
</SettingsSection>

<style>
	.disabled-pill {
		padding: 4px 8px;
		border-radius: 999px;
		background: var(--bg-2);
		color: var(--text-3);
		font-size: 12px;
	}
</style>
