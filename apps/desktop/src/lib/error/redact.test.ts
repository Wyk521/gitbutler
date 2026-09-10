import { describe, expect, it } from "vitest";
import { redactSensitiveText } from "$lib/error/redact";

describe("redactSensitiveText", () => {
	it("removes URL, bearer, and key/value credentials", () => {
		const redacted = redactSensitiveText(
			"https://alice:secret@example.com/api Bearer abc.def token=raw-token password: 'hunter2'",
		);
		expect(redacted).not.toContain("secret");
		expect(redacted).not.toContain("abc.def");
		expect(redacted).not.toContain("raw-token");
		expect(redacted).not.toContain("hunter2");
		expect(redacted.match(/\[已脱敏\]/g)?.length).toBe(4);
	});
});
