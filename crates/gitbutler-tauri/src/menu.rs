#[cfg(target_os = "macos")]
use anyhow::Context as _;
#[cfg(target_os = "macos")]
use tauri::menu::AboutMetadata;
use tauri::{
    AppHandle, Emitter, EventTarget, Runtime, WebviewWindow,
    menu::{Menu, MenuEvent, MenuItemBuilder, PredefinedMenuItem, Submenu, SubmenuBuilder},
};

static SHORTCUT_EVENT: &str = "menu://shortcut";

pub fn build<R: Runtime>(handle: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    #[cfg(not(any(feature = "disable-auto-updates", feature = "offline")))]
    let check_for_updates =
        MenuItemBuilder::with_id("global/update", "Check for updates…").build(handle)?;

    #[cfg(target_os = "macos")]
    let app_name = handle
        .config()
        .product_name
        .clone()
        .context("App name not defined.")?;

    #[cfg(target_os = "macos")]
    let settings_menu = MenuItemBuilder::with_id("global/settings", "设置")
        .accelerator("CmdOrCtrl+,")
        .build(handle)?;

    #[cfg(target_os = "macos")]
    let mac_menu = {
        #[cfg_attr(feature = "disable-auto-updates", allow(unused_mut))]
        let mut menu = SubmenuBuilder::new(handle, app_name)
            .about(Some(AboutMetadata::default()))
            .separator()
            .item(&settings_menu);

        #[cfg(not(any(feature = "disable-auto-updates", feature = "offline")))]
        {
            menu = menu.item(&check_for_updates);
        }
        menu.separator()
            .services()
            .separator()
            .hide()
            .hide_others()
            .show_all()
            .separator()
            .quit()
            .build()?
    };

    let file_menu = &SubmenuBuilder::new(handle, "文件")
        .items(&[
            &MenuItemBuilder::with_id("file/add-local-repo", "添加本地仓库…")
                .accelerator("CmdOrCtrl+O")
                .build(handle)?,
            #[cfg(not(feature = "offline"))]
            &MenuItemBuilder::with_id("file/clone-repo", "克隆仓库…")
                .accelerator("CmdOrCtrl+Shift+O")
                .build(handle)?,
            &PredefinedMenuItem::separator(handle)?,
            &MenuItemBuilder::with_id("file/create-branch", "创建分支…")
                .accelerator("CmdOrCtrl+B")
                .build(handle)?,
            &MenuItemBuilder::with_id("file/create-dependent-branch", "创建依赖分支…")
                .accelerator("CmdOrCtrl+Shift+B")
                .build(handle)?,
            &PredefinedMenuItem::separator(handle)?,
        ])
        .build()?;

    #[cfg(target_os = "macos")]
    file_menu.append(&PredefinedMenuItem::close_window(handle, None)?)?;

    if cfg!(not(target_os = "macos")) {
        file_menu.append_items(&[&PredefinedMenuItem::quit(handle, None)?])?;
        #[cfg(not(any(feature = "disable-auto-updates", feature = "offline")))]
        file_menu.append_items(&[&check_for_updates])?;
    }

    #[cfg(not(target_os = "linux"))]
    let edit_menu = &Submenu::new(handle, "编辑", true)?;

    #[cfg(not(target_os = "linux"))]
    {
        edit_menu.append_items(&[
            &PredefinedMenuItem::cut(handle, None)?,
            &PredefinedMenuItem::copy(handle, None)?,
            &PredefinedMenuItem::paste(handle, None)?,
        ])?;
    }

    let view_menu = &Submenu::new(handle, "视图", true)?;

    #[cfg(target_os = "macos")]
    view_menu.append(&PredefinedMenuItem::fullscreen(handle, None)?)?;
    view_menu.append_items(&[
        &MenuItemBuilder::with_id("view/switch-theme", "切换主题")
            .accelerator("CmdOrCtrl+T")
            .build(handle)?,
        &MenuItemBuilder::with_id("view/toggle-sidebar", "显示/隐藏侧栏")
            .accelerator("CmdOrCtrl+\\")
            .build(handle)?,
        &PredefinedMenuItem::separator(handle)?,
        &MenuItemBuilder::with_id("view/zoom-in", "放大")
            .accelerator("CmdOrCtrl+=")
            .build(handle)?,
        &MenuItemBuilder::with_id("view/zoom-out", "缩小")
            .accelerator("CmdOrCtrl+-")
            .build(handle)?,
        &MenuItemBuilder::with_id("view/zoom-reset", "重置缩放")
            .accelerator("CmdOrCtrl+0")
            .build(handle)?,
        &PredefinedMenuItem::separator(handle)?,
    ])?;

    #[cfg(any(debug_assertions, feature = "devtools"))]
    view_menu.append_items(&[
        &MenuItemBuilder::with_id("view/devtools", "开发者工具")
            .accelerator("CmdOrCtrl+Shift+C")
            .build(handle)?,
        &MenuItemBuilder::with_id("view/reload", "重新加载界面")
            .accelerator("CmdOrCtrl+R")
            .build(handle)?,
    ])?;

    let mut project_menu_builder = SubmenuBuilder::new(handle, "项目")
        .item(
            &MenuItemBuilder::with_id("project/history", "操作历史")
                .accelerator("CmdOrCtrl+Shift+H")
                .build(handle)?,
        )
        .separator()
        .text("project/open-in-vscode", "在编辑器中打开")
        .text("project/open-in-terminal", "在终端中打开");

    #[cfg(target_os = "macos")]
    {
        project_menu_builder = project_menu_builder.text("project/show-in-finder", "在访达中显示");
    }

    #[cfg(target_os = "windows")]
    {
        project_menu_builder =
            project_menu_builder.text("project/show-in-finder", "在资源管理器中显示");
    }

    #[cfg(target_os = "linux")]
    {
        project_menu_builder =
            project_menu_builder.text("project/show-in-finder", "在文件管理器中显示");
    }

    let project_menu = &project_menu_builder
        .separator()
        .text("project/settings", "项目设置")
        .build()?;

    #[cfg(target_os = "macos")]
    let window_menu = &SubmenuBuilder::new(handle, "窗口")
        .items(&[
            &PredefinedMenuItem::minimize(handle, None)?,
            &PredefinedMenuItem::maximize(handle, None)?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::close_window(handle, None)?,
        ])
        .build()?;

    let help_menu_builder = SubmenuBuilder::new(handle, "帮助")
        .text("help/open-logs-folder", "打开日志文件夹")
        .text("help/open-config-folder", "打开配置文件夹")
        .text("help/open-cache-folder", "打开缓存文件夹");

    #[cfg(not(feature = "offline"))]
    let help_menu_builder = help_menu_builder
        .separator()
        .text("help/documentation", "在线文档")
        .text("help/debugging-guide", "调试指南")
        .text("help/github", "源代码")
        .text("help/release-notes", "发行说明")
        .separator()
        .text("help/share-debug-info", "分享调试信息…")
        .text("help/report-issue", "提交问题")
        .separator()
        .text("help/discord", "Discord")
        .text("help/youtube", "YouTube")
        .text("help/bluesky", "Bluesky")
        .text("help/x", "X");

    let help_menu = help_menu_builder
        .separator()
        .item(
            &MenuItemBuilder::with_id(
                "help/version",
                format!("版本 {}", handle.package_info().version),
            )
            .enabled(false)
            .build(handle)?,
        )
        .build()?;

    Menu::with_items(
        handle,
        &[
            #[cfg(target_os = "macos")]
            &mac_menu,
            file_menu,
            #[cfg(not(target_os = "linux"))]
            edit_menu,
            view_menu,
            project_menu,
            #[cfg(target_os = "macos")]
            window_menu,
            &help_menu,
        ],
    )
}

pub fn handle_event(webview: &WebviewWindow, event: &MenuEvent) {
    if event.id() == "file/add-local-repo" {
        emit(webview, "menu://shortcut", "add-local-repo");
        return;
    }

    #[cfg(not(feature = "offline"))]
    if event.id() == "file/clone-repo" {
        emit(webview, SHORTCUT_EVENT, "clone-repo");
        return;
    }

    if event.id() == "file/create-branch" {
        emit(webview, SHORTCUT_EVENT, "create-branch");
        return;
    }

    if event.id() == "file/create-dependent-branch" {
        emit(webview, SHORTCUT_EVENT, "create-dependent-branch");
        return;
    }

    #[cfg(any(debug_assertions, feature = "devtools"))]
    {
        if event.id() == "view/devtools" {
            if webview.is_devtools_open() {
                webview.close_devtools();
            } else {
                webview.open_devtools();
            }
            return;
        }
    }

    if event.id() == "view/switch-theme" {
        emit(webview, SHORTCUT_EVENT, "switch-theme");
        return;
    }

    if event.id() == "view/toggle-sidebar" {
        emit(webview, SHORTCUT_EVENT, "toggle-sidebar");
        return;
    }

    if event.id() == "view/reload" {
        emit(webview, SHORTCUT_EVENT, "reload");
        return;
    }

    if event.id() == "view/zoom-in" {
        emit(webview, SHORTCUT_EVENT, "zoom-in");
        return;
    }

    if event.id() == "view/zoom-out" {
        emit(webview, SHORTCUT_EVENT, "zoom-out");
        return;
    }

    if event.id() == "view/zoom-reset" {
        emit(webview, SHORTCUT_EVENT, "zoom-reset");
        return;
    }

    #[cfg(not(feature = "offline"))]
    if event.id() == "help/share-debug-info" {
        emit(webview, SHORTCUT_EVENT, "share-debug-info");
        return;
    }

    if event.id() == "project/history" {
        emit(webview, SHORTCUT_EVENT, "history");
        return;
    }

    if event.id() == "project/open-in-vscode" {
        emit(webview, SHORTCUT_EVENT, "open-in-vscode");
        return;
    }

    if event.id() == "project/open-in-terminal" {
        emit(webview, SHORTCUT_EVENT, "open-in-terminal");
        return;
    }

    if event.id() == "project/show-in-finder" {
        emit(webview, SHORTCUT_EVENT, "show-in-finder");
        return;
    }

    if event.id() == "project/settings" {
        emit(webview, SHORTCUT_EVENT, "project-settings");
        return;
    }

    if event.id() == "global/settings" {
        emit(webview, SHORTCUT_EVENT, "global-settings");
        return;
    }

    #[cfg(not(any(feature = "disable-auto-updates", feature = "offline")))]
    if event.id() == "global/update" {
        emit(webview, SHORTCUT_EVENT, "update");
        return;
    }

    if event.id() == "help/open-logs-folder" {
        if let Err(err) = crate::debug::open_logs_folder() {
            tracing::error!(error = ?err, "failed to open logs folder");
        }
        return;
    }

    if event.id() == "help/open-config-folder" {
        if let Err(err) = crate::debug::open_config_folder() {
            tracing::error!(error = ?err, "failed to open config folder");
        }
        return;
    }

    if event.id() == "help/open-cache-folder" {
        if let Err(err) = crate::debug::open_cache_folder() {
            tracing::error!(error = ?err, "failed to open cache folder");
        }
        return;
    }

    #[cfg(not(feature = "offline"))]
    'open_link: {
        let result = match event.id().0.as_str() {
            "help/documentation" => open::that("https://docs.gitbutler.com"),
            "help/debugging-guide" => {
                open::that("https://docs.gitbutler.com/development/debugging")
            }
            "help/github" => open::that("https://github.com/gitbutlerapp/gitbutler"),
            "help/release-notes" => {
                open::that("https://github.com/gitbutlerapp/gitbutler/releases")
            }
            "help/report-issue" => {
                open::that("https://github.com/gitbutlerapp/gitbutler/issues/new/choose")
            }
            "help/discord" => open::that("https://discord.com/invite/MmFkmaJ42D"),
            "help/youtube" => open::that("https://www.youtube.com/@gitbutlerapp"),
            "help/bluesky" => open::that("https://bsky.app/profile/gitbutler.com"),
            "help/x" => open::that("https://x.com/gitbutler"),
            _ => break 'open_link,
        };

        if let Err(err) = result {
            tracing::error!(error = ?err, "failed to open url for {}", event.id().0);
        }

        return;
    }

    tracing::debug!("unhandled menu event: {}", event.id().0);
}

fn emit<R: Runtime>(window: &WebviewWindow<R>, event: &str, shortcut: &str) {
    if let Err(err) = window.emit_to(EventTarget::window(window.label()), event, shortcut) {
        tracing::error!(error = ?err, "failed to emit event");
    }
}
