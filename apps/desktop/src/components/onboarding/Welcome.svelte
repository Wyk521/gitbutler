<script lang="ts">
	import WelcomeAction from "$components/onboarding/WelcomeAction.svelte";
	import newProjectSvg from "$lib/assets/welcome/new-local-project.svg?raw";
	import { handleAddProjectOutcome } from "$lib/project/project";
	import { PROJECTS_SERVICE } from "$lib/project/projectsService";
	import { inject } from "@gitbutler/core/context";
	import { TestId } from "@gitbutler/ui";

	const projectsService = inject(PROJECTS_SERVICE);
	const serverCapabilitiesQuery = $derived(projectsService.serverCapabilities());
	const canAddProjects = $derived(serverCapabilitiesQuery.response?.canAddProjects ?? true);

	let newProjectLoading = $state(false);
	let directoryInputElement = $state<HTMLInputElement | undefined>();

	async function onNewProject() {
		newProjectLoading = true;
		try {
			const testDirectoryPath = directoryInputElement?.value;
			const outcome = await projectsService.addProject(testDirectoryPath ?? "");

			if (outcome) {
				handleAddProjectOutcome(outcome);
			}
		} catch (e: unknown) {
		} finally {
			newProjectLoading = false;
		}
	}

</script>

<div class="welcome" data-testid={TestId.WelcomePage}>
	<h1 class="welcome-title text-serif-42">欢迎使用 RepoScope Desktop</h1>
	<div class="welcome__actions">
		<div class="welcome__actions--repo">
			<input
				type="text"
				hidden
				bind:this={directoryInputElement}
				data-testid="test-directory-path"
			/>
			{#if canAddProjects}
				<WelcomeAction
					title="添加本机仓库"
					loading={newProjectLoading}
					onclick={onNewProject}
					dimMessage
					testId={TestId.WelcomePageAddLocalProjectButton}
				>
					{#snippet icon()}
						{@html newProjectSvg}
					{/snippet}
					{#snippet message()}
						请选择有效的本机 Git 仓库
					{/snippet}
				</WelcomeAction>
			{/if}
		</div>
	</div>
</div>

<style lang="postcss">
	.welcome {
		width: 100%;
	}

	.welcome-title {
		color: var(--text-1);
		line-height: 1;
	}

	.welcome__actions {
		display: flex;
		flex-direction: column;
		margin-top: 32px;
		gap: 8px;
	}

	.welcome__actions--repo {
		display: flex;
		gap: 8px;
	}

	.links {
		display: flex;
		margin-top: 20px;
		padding: 28px;
		gap: 56px;
		border-radius: var(--radius-m);
		background: var(--bg-mute);
	}

	.links__section {
		display: flex;
		flex-direction: column;
		gap: 20px;
	}

	.education-links {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		margin-left: -6px;
		gap: 6px;
	}

	.community-links {
		display: grid;
		grid-template-columns: repeat(2, 1fr);
		column-gap: 12px;
		row-gap: 4px;
		max-width: 192px;
		margin-left: -6px;
	}

	/* SMALL ILLUSTRATIONS */
</style>
