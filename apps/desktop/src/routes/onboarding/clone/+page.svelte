<svelte:head><title>RepoScope Desktop · 本机模式</title></svelte:head>

<div class="offline-clone">
	<h1>本版本仅支持本机仓库</h1>
	<p>严格离线模式不会访问远程地址。请返回欢迎页并选择已经存在于本机的 Git 仓库。</p>
	<a href="/onboarding">返回选择本机仓库</a>
</div>

<style>
	.offline-clone { display: grid; place-items: center; align-content: center; min-height: 100%; padding: 32px; color: var(--text-1); text-align: center; }
	.offline-clone p { max-width: 420px; color: var(--text-2); font-size: 13px; line-height: 1.6; }
	.offline-clone a { margin-top: 16px; color: var(--text-1); }
</style>
