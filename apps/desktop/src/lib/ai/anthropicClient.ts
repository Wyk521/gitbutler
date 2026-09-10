import {
	SHORT_DEFAULT_COMMIT_TEMPLATE,
	SHORT_DEFAULT_BRANCH_TEMPLATE,
	SHORT_DEFAULT_PR_TEMPLATE,
} from "$lib/ai/prompts";
import { type AIClient, type AnthropicModelName, type Prompt, type AIEvalOptions } from "$lib/ai/types";

export class AnthropicAIClient implements AIClient {
	defaultCommitTemplate = SHORT_DEFAULT_COMMIT_TEMPLATE;
	defaultBranchTemplate = SHORT_DEFAULT_BRANCH_TEMPLATE;
	defaultPRTemplate = SHORT_DEFAULT_PR_TEMPLATE;

	constructor(_apiKey: string, _modelName: AnthropicModelName) {}

	async evaluate(_prompt: Prompt, _options?: AIEvalOptions): Promise<string> {
		throw new Error("RepoScope Desktop 离线模式已禁用 AI 功能");
	}
}
