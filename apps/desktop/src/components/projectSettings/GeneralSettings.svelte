<script lang="ts">
	import DetailsForm from "$components/projectSettings/DetailsForm.svelte";
	import GerritForm from "$components/projectSettings/GerritForm.svelte";
	import RemoveProjectForm from "$components/projectSettings/RemoveProjectForm.svelte";
	import RepoScopeAnalysisSettings from "$components/projectSettings/RepoScopeAnalysisSettings.svelte";
	import { Spacer } from "@gitbutler/ui";

	const { projectId }: { projectId: string } = $props();
</script>

<DetailsForm {projectId} />
<GerritForm {projectId} />
<RepoScopeAnalysisSettings {projectId} />
<Spacer />
<RemoveProjectForm {projectId} />
