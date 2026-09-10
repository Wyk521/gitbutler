/**
 * Commands which could contact a forge, cloud service, remote Git transport,
 * updater or AI provider.  The check is deliberately performed at the IPC
 * boundary as a second line of defence behind the Rust command registration
 * gates, so a stale UI bundle cannot accidentally re-enable networking.
 */
const BLOCKED_PREFIXES = [
	"forge_",
	"github_",
	"gitlab_",
	"bitbucket_",
	"list_reviews",
	"get_review",
	"update_review",
	"publish_review",
	"merge_review",
	"list_ci_checks",
	"workspace_fetch",
	"workspace_integrate",
	"workspace_branch_and_ancestors_push",
	"git_test_fetch",
	"git_test_push",
	"fetch_from_remotes",
	"add_remote",
	"git_clone",
	"clone_",
	"resolve_commit_conflicts_ai",
	"branch_land",
	"install_cli",
	"get_anonymous_graph",
	"update_telemetry",
	"update_fetch",
	"update_reviews",
] as const;

/** Return whether a command is forbidden in the offline desktop build. */
export function isOfflineCommand(command: string): boolean {
	// Vitest keeps the upstream command fixtures available for deterministic
	// unit tests; every actual desktop build (including development builds)
	// remains offline unless a developer explicitly opts out with the local
	// `VITE_REPOSCOPE_OFFLINE=false` override.
	if (import.meta.env.VITEST || import.meta.env.VITE_REPOSCOPE_OFFLINE === "false") return false;
	return BLOCKED_PREFIXES.some((prefix) => command.startsWith(prefix));
}
