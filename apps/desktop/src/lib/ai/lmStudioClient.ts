import {
	SHORT_DEFAULT_BRANCH_TEMPLATE,
	SHORT_DEFAULT_COMMIT_TEMPLATE,
	SHORT_DEFAULT_PR_TEMPLATE,
} from "$lib/ai/prompts";
import type { Prompt, AIClient, AIEvalOptions } from "$lib/ai/types";

export const LM_STUDIO_DEFAULT_ENDPOINT = "http://127.0.0.1:1234";
export const LM_STUDIO_DEFAULT_MODEL_NAME = "default";

/**
 * LMStudioClient implements the AIClient interface for LM Studio servers.
 * LM Studio provides an OpenAI-compatible API at the /v1/chat/completions endpoint.
 */
export class LMStudioClient implements AIClient {
	defaultCommitTemplate = SHORT_DEFAULT_COMMIT_TEMPLATE;
	defaultBranchTemplate = SHORT_DEFAULT_BRANCH_TEMPLATE;
	defaultPRTemplate = SHORT_DEFAULT_PR_TEMPLATE;

	constructor(_endpoint: string, _modelName: string) {}

	async evaluate(_prompt: Prompt, _options?: AIEvalOptions): Promise<string> {
		throw new Error("RepoScope Desktop 离线模式已禁用 AI 功能");
	}
}
