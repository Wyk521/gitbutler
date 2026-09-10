<script lang="ts">
	import { page } from "$app/state";
	import RepoScopeCommitDetail from "$components/reposcope/RepoScopeCommitDetail.svelte";

	const projectId = $derived(page.params.projectId!);
	const oid = $derived(page.params.oid!);
</script>

<RepoScopeCommitDetail {projectId} {oid} />
