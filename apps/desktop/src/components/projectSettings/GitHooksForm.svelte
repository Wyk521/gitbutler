<script lang="ts">
	import SettingsSection from "$components/shared/SettingsSection.svelte";
	import { CardGroup } from "@gitbutler/ui";

	const { projectId }: { projectId: string } = $props();
	void projectId;
</script>

<SettingsSection>
	<CardGroup>
		<CardGroup.Item>
			{#snippet title()}仓库自定义 Hooks{/snippet}
			{#snippet caption()}严格离线版本不会执行仓库中的 pre-commit、post-commit、commit-msg、pre-push 或 Husky 脚本；GitButler 内置工作区保护仍然有效。{/snippet}
			{#snippet actions()}<span class="disabled-pill">已禁用</span>{/snippet}
		</CardGroup.Item>
	</CardGroup>
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
