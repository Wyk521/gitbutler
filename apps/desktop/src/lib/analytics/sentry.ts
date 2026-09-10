/**
 * RepoScope Desktop deliberately has no error telemetry transport. These
 * compatibility functions keep shared GitButler services linkable without
 * pulling Sentry into the runtime bundle.
 */

export function initSentry(): void {
	// Intentionally empty: release builds never send diagnostics off device.
}

export function setSentryUser(_user: { id: number; email?: string; name?: string }): void {
	// Intentionally empty.
}

export function resetSentry(): void {
	// Intentionally empty.
}
