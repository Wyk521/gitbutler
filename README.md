<div align="center">
  
  <img align="center" width="100px" src="crates/gitbutler-tauri/icons/release/icon.png" alt="RepoScope Desktop logo" />
  <br />

  <h1 align="center">RepoScope Desktop</h1>
  
  <p align="center">
   <b>本机仓库洞察与 Git 工作区</b>
   <br/>
   RepoScope Desktop 基于 GitButler 的本地工作区能力，提供严格离线的仓库分析、审计线索和诊断。
    <br />
    <br />
    <a href="https://github.com/Wyk521/gitbutler">代码仓库</a>
    <span>&nbsp;&nbsp;•&nbsp;&nbsp;</span>
    <a href="REPOSCOPE_DESKTOP.md">实现说明</a>
  </p>

  <br/>

  <br/>

[![TWEET][s1]][l1] [
![BLUESKY][s8]][l8] [![DISCORD][s2]][l2]

[![CI][s0]][l0] [![INSTA][s3]][l3] [![YOUTUBE][s5]][l5] [![DEEPWIKI][s7]][l7]

[s0]: https://github.com/gitbutlerapp/gitbutler/actions/workflows/push.yaml/badge.svg
[l0]: https://github.com/gitbutlerapp/gitbutler/actions/workflows/push.yaml
[s1]: https://img.shields.io/badge/Twitter-black?logo=x&logoColor=white
[l1]: https://twitter.com/intent/follow?screen_name=gitbutler
[s2]: https://img.shields.io/discord/1060193121130000425?label=Discord&color=5865F2
[l2]: https://discord.gg/MmFkmaJ42D
[s3]: https://img.shields.io/badge/Instagram-E4405F?logo=instagram&logoColor=white
[l3]: https://www.instagram.com/gitbutler/
[s5]: https://img.shields.io/youtube/channel/subscribers/UCEwkZIHGqsTGYvX8wgD0LoQ
[l5]: https://www.youtube.com/@gitbutlerapp
[s7]: https://deepwiki.com/badge.svg
[l7]: https://deepwiki.com/gitbutlerapp/gitbutler
[s8]: https://img.shields.io/badge/Bluesky-0285FF?logo=bluesky&logoColor=fff
[l8]: https://bsky.app/profile/gitbutler.com

</div>

<br/>

RepoScope Desktop 是面向 Windows 10/11 x64 的本机 Git 工作区和仓库洞察工具。GitButler 的并行分支、堆叠分支、提交编辑、冲突处理和撤销历史继续保留；RepoScope 分析引擎完全使用 Rust，并把结果保存到独立的 `reposcope.sqlite`。

它提供堆叠分支、并行分支、无限撤销、提交拆分/合并/修改，以及总览、活跃度、热点、耦合、所有权、代码年龄、Bus Factor、交付溯源、地域、依赖和仓库诊断。

应用只读取本机 Git 对象和工作目录，不启动 Python、Node 服务，不连接网络，也不修改 Git refs。

## Main Features

Why use GitButler instead of vanilla Git? What a great question.

- **Stacked Branches** ([gui](https://docs.gitbutler.com/features/branch-management/stacked-branches), [cli](https://docs.gitbutler.com/cli-guides/cli-tutorial/branching-and-commiting#stacked-branches))
  - Effortlessly create branches stacked on other branches. Amend or edit any commit easily with automatic restacking.
- **Parallel Branches** ([gui](https://docs.gitbutler.com/features/branch-management/virtual-branches), [cli](https://docs.gitbutler.com/cli-guides/cli-tutorial/branching-and-commiting#parallel-branches))
  - Organize work on multiple branches simultaneously, rather than constantly switching branches.
- **Easy Commit Management** ([gui](https://docs.gitbutler.com/features/branch-management/commits), [cli](https://docs.gitbutler.com/cli-guides/cli-tutorial/rubbing))
  - Uncommit, reword, amend, move, split and squash commits by dragging and dropping or simple CLI commands. Forget about `rebase -i`, you don't need it anymore.
- **Undo Timeline** ([gui](https://docs.gitbutler.com/features/timeline), [cli](https://docs.gitbutler.com/cli-guides/cli-tutorial/operations-log))
  - Logs all operations and changes and allows you to easily undo or revert any operation.
- **First Class Conflicts** ([gui](https://docs.gitbutler.com/overview#conflicting-branches), [cli](https://docs.gitbutler.com/cli-guides/cli-tutorial/conflict-resolution))
  - Rebases always succeed. Commits can be marked as conflicted and resolved at any time, in any order.
- **仓库洞察**
  - 在本机生成可追溯的分析批次，支持分页、筛选、提交/文件/证据下钻，并保留旧结果直到新批次成功。
- **严格离线**
  - 发布包不注册 Forge、登录、AI、遥测、更新、fetch、push、clone 或外部 URL 命令；源码预览和错误信息进入数据库前自动脱敏。

## Tech

RepoScope Desktop 是基于 [Tauri](https://tauri.app/) 的 Windows 桌面应用，界面使用 [Svelte](https://svelte.dev/) 和 TypeScript，分析与 Git 工作区后端使用 [Rust](https://www.rust-lang.org/)。

发布包不携带 `but` CLI 或 Askpass 侧车；GitButler 来源、版权和 FSL-1.1-MIT 许可入口保留在应用关于页和 `LICENSE.md`。

## Documentation

You can find our end user documentation at: <https://docs.gitbutler.com>

## Bugs and Feature Requests

If you have a bug or feature request, feel free to open an [issue](https://github.com/gitbutlerapp/gitbutler/issues/new),
or [join our Discord server](https://discord.gg/MmFkmaJ42D).

## License

The TLDR is that GitButler is under a [Fair Source](https://fair.io/) software license, meaning that you can use it, view the source, contribute, etc. You just can't build a competitor with it. It also becomes MIT after 2 years. So, MIT with an expiring non-compete clause.

## Contributing

So you want to help out? Please check out the [CONTRIBUTING.md](CONTRIBUTING.md)
document.

If you want to skip right to getting the code to actually compile, take a look
at the [DEVELOPMENT.md](DEVELOPMENT.md) file.

### Contributors

<a href="https://github.com/gitbutlerapp/gitbutler/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=gitbutlerapp/gitbutler" />
</a>
