import {
	SHORT_DEFAULT_BRANCH_TEMPLATE,
	SHORT_DEFAULT_COMMIT_TEMPLATE,
	SHORT_DEFAULT_PR_TEMPLATE,
} from "$lib/ai/prompts";
import type {
	OpenAIModelName,
	OpenRouterModelName,
	Prompt,
	AIClient,
	AIEvalOptions,
} from "$lib/ai/types";

export class OpenAIClient implements AIClient {
	defaultCommitTemplate = SHORT_DEFAULT_COMMIT_TEMPLATE;
	defaultBranchTemplate = SHORT_DEFAULT_BRANCH_TEMPLATE;
	defaultPRTemplate = SHORT_DEFAULT_PR_TEMPLATE;

	constructor(
		_openAIKey: string,
		_modelName: OpenAIModelName | OpenRouterModelName,
		_baseURL: string | undefined,
	) {}

	async evaluate(_prompt: Prompt, _options?: AIEvalOptions): Promise<string> {
		throw new Error("RepoScope Desktop 离线模式已禁用 AI 功能");
	}
}
