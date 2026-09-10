/**
 * Redact credentials before an error, URL, or repository-derived value crosses
 * into a toast, console, or local log.  This is intentionally dependency-free
 * so it can be used by the backend adapters without creating an import cycle.
 */
export function redactSensitiveText(value: string): string {
	const normalized = value.replace(/[\u0000\r\n]/g, " ");
	const withUrlCredentials = normalized.replace(
		/([a-z][a-z0-9+.-]*:\/\/)[^/\s:@]+(?::[^/\s@]*)?@/gi,
		"$1[已脱敏]@",
	);
	const withJdbcCredentials = withUrlCredentials.replace(
		/(jdbc:[^:\s]+(?::[^:\s]+)*:)[^/\s:@]+\/[^@\s]+@/gi,
		"$1[已脱敏]@",
	);
	const withBearer = withJdbcCredentials.replace(
		/\bBearer\s+[A-Za-z0-9._~+/=-]+/gi,
		"Bearer [已脱敏]",
	);
	return withBearer.replace(
		/["']?(user(?:name)?|login|password|passwd|pwd|token|secret|api[_-]?key|access[_-]?token|client[_-]?secret|authorization|private[_-]?key)["']?\s*([:=])\s*(?:"[^"]*"|'[^']*'|[^,;\s}\]]+)/gi,
		"$1$2[已脱敏]",
	);
}
