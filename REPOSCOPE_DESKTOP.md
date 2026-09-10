# RepoScope Desktop

RepoScope Desktop 是基于 GitButler `687db344ec4e7828b355a8496e935bad1b5516bb` 的本地离线重构分支。GitButler 的本地工作区、并行/堆叠分支、提交编辑、冲突处理和操作历史仍由原有模块负责；RepoScope 分析只读 Git 对象和工作目录，不写 refs，也不进入 GitButler 撤销时间线。

## 当前实现

- `crates/but-reposcope` 是独立的 Rust 分析引擎：refs/提交遍历、首父 diff、重命名链、工作树扫描、语言/活跃度/热点/耦合、Blame 所有权、代码年龄、Bus Factor、地域/依赖/交付/外部系统线索和仓库诊断。
- `reposcope.sqlite` 与 GitButler 的 `but.sqlite` 分离，使用 WAL、外键和 busy timeout。结果按 `analysis_run_id` 批次写入，只有完整事务提交后才切换 `active_run_id`；可选审计阶段失败会保留核心结果并标记 `partial`。
- 运行失败信息在写入 `analysis_runs.error` 前再次执行 URL 用户信息、JDBC、Bearer 和常见密钥脱敏，避免仓库内容或解析器错误把凭据留在本地缓存。
- `crates/but-api/src/reposcope.rs` 暴露分析任务、核心报告、风险指标、证据报告、文件预览、诊断、refs 和项目级分析设置。文件预览支持当前工作树、裸仓库 HEAD 和指定历史提交，并在返回前执行路径、对象类型、二进制和凭据脱敏校验；洞察报告中的文件与证据行可跳转到只读预览并定位行号。分析进度通过 `project://{projectId}/reposcope-analysis` 事件发送。
- Blame 所有权阶段按设置启动 1–8 个独立只读 Git worker，主线程合并结果、发送进度并响应取消；源码预览对工作树和历史 blob 统一执行 20 MB 输入上限，最终返回仍限制为 150 万字符。
- `apps/desktop/src/routes/[projectId]/insights/` 提供中文“仓库洞察”入口；侧栏保留工作区、分支、操作历史，并复用 GitButler 原生 Git 操作页面。分析进度由 `project://{projectId}/reposcope-analysis` 事件驱动，前端不再使用定时轮询。
- Tauri 默认启用 `offline` feature：远程、Forge、登录、AI、遥测、上传、更新、clone/fetch/push 和 HTTP(S)/mailto URL 打开命令不注册；仅保留经过协议和主机校验的本机文件/编辑器 URI。release CSP 只允许打包资源、Tauri IPC 和本地数据。开发包也默认拒绝这些命令（Vitest 仅保留测试夹具）。仓库自定义 hooks 在离线构建中不执行，诊断页仍只读列出 hooks；离线构建还不会启动 Tokio 本机调试 HTTP 监听。桌面包已移除 Anthropic/OpenAI/Ollama 在线 SDK，兼容 AI 客户端保留为明确失败的占位实现。
- 发布配置不再声明 Tauri `externalBin`，发布前置脚本不编译或注入 `but`、Git Askpass 侧车，Linux RPM 也不创建 `/usr/bin/but` 符号链接；主程序输出名为 `reposcope-desktop`（Windows 为 `reposcope-desktop.exe`）。
- 添加项目后首次打开会自动排队分析；异常退出后的运行会标记为 `interrupted` 并在再次打开项目时恢复一次扫描。洞察页以分析事件为主，仅在打开、完成和 watcher 标记过期时读取状态。
- `but-ts` 已将 RepoScope Rust DTO 写入 `packages/but-sdk/src/generated/{linear,graph}/index.d.ts`，前端通过 SDK 类型导入；通用 `Page<T>` 因生成器只输出具体 schema，仍由领域层保留一个轻量泛型适配器。N-API 函数声明需在 Node 24 环境重新构建 N-API 产物后再生成。

## 本地开发状态

当前工作树分支为 `reposcope-desktop`，`origin` 已配置为用户 fork `https://github.com/你的 fork 用户名/gitbutler.git`，`upstream` 仍指向 GitButler 主仓库。Windows 构建入口为 `scripts/build-reposcope-windows.ps1`，会生成未签名的 x64 MSI 内部安装包。

旧版 `旧版 RepoScope 目录` 没有 `.git`，本次仅作为迁移参考，未删除、未改写，也没有迁移旧 SQLite。

## 构建与测试

Rust 与前端 SDK 类型生成完成后执行：

```text
cargo test -p but-reposcope
cargo test -p but-api reposcope
cargo check -p gitbutler-tauri --all-targets
cargo clippy --workspace --all-targets
cargo fmt --check
pnpm build:sdk
pnpm --filter @gitbutler/desktop test
pnpm --filter @gitbutler/desktop check
```

- Rust 核心、`but-api`（`tauri,legacy`）和 Tauri all-targets 已有通过基线；`but-reposcope` 现包含 30 个 fixture/单元测试，覆盖 Unicode、重命名、空/裸仓库、工作树、Blame、批次原子性、取消、锁、脱敏、边界和终态状态机。改动后的完整检查命令保留在上方，Windows 主机构建与前端检查统一由 `scripts/build-reposcope-windows.ps1` 入口执行，避免开发机与 WSL 的工具链差异。
- SDK 构建会先生成 Windows N-API 产物，再运行 `but-ts` 写入两个前端 flavor 的 DTO 和 API 声明；发布包默认使用 `offline` feature，所有外连入口在 Rust 注册层、前端 IPC 层和 release CSP 三层关闭。

## Windows 构建

在 Windows PowerShell 中从仓库根目录执行：

```powershell
.\scripts\build-reposcope-windows.ps1 -Version 0.1.0
```

首次没有安装依赖时追加 `-InstallDependencies`。脚本会生成 SDK 类型、构建生产前端和离线 Tauri MSI，并把安装包复制到 `dist\windows`。

## 许可证和发布边界

GitButler 原 `LICENSE.md`、版权和来源文件保持在仓库中；关于页已保留“基于 GitButler”、版权所有者和打包的完整 FSL-1.1-MIT 入口（`apps/desktop/static/licenses/FSL-1.1-MIT.txt`）。当前实现定位个人/内部使用。准备商业销售、SaaS 或对外分发前，必须重新核对目标 GitButler 提交的许可证状态或取得单独商业授权。没有签名证书时，Windows 包应标记为“未签名内部版本”。
