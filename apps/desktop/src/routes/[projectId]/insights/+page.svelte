<script lang="ts">
	import { page } from "$app/state";
	import RepoScopeInsights from "$components/reposcope/RepoScopeInsights.svelte";

	const projectId = $derived(page.params.projectId!);
</script>

<RepoScopeInsights {projectId} />
