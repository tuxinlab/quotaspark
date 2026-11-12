mod cc_sync;
mod commands;
mod engine;
mod quota;
mod scheduler;
mod state;
mod store;

use state::{AppState, Inner};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use store::Config;
use tauri::Manager;
// 仅 release 的自启默认开启/路径自愈用到（debug 下相关代码整体 cfg 掉）
#[cfg(not(debug_assertions))]
use tauri_plugin_autostart::ManagerExt;

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;

    let open = MenuItem::with_id(app, "open", "显示主窗口", true, None::<&str>)?;
    let act = MenuItem::with_id(app, "activate_all", "立即全部激活", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &act, &quit])?;

    TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().expect("missing icon").clone())
        .tooltip("额度火花 QuotaSpark")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
            }
            "activate_all" => {
                let shared = app.state::<AppState>().0.clone();
                commands::activate_all_spawn(app.clone(), shared);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config_path = store::data_dir().join("config.json");
    let config = Config::load(&config_path);
    let logs = store::load_log_tail(200);
    let app_state = AppState(Arc::new(Mutex::new(Inner {
        config,
        config_path,
        logs,
        quota: HashMap::new(),
    })));

    tauri::Builder::default()
        // 单实例锁：防止两个实例共用一份 config.json 互相覆盖（旧实例的
        // 内存状态会把已删除的供应商写回磁盘）。必须是第一个注册的插件。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            // 注册自启时附带 --minimized：开机拉起后静默进托盘，不弹主窗口
            Some(vec!["--minimized"]),
        ))
        .plugin(tauri_plugin_opener::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::get_providers,
            commands::get_logs,
            commands::sync_from_cc,
            commands::save_provider,
            commands::delete_provider,
            commands::reorder_providers,
            commands::set_schedule,
            commands::activate_now,
            commands::query_quota,
            commands::get_autostart,
            commands::set_autostart,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // 开机自启默认开启：标记缺失（旧配置/全新安装）时补开并写标记，
            // 之后以顶栏开关为准，用户手动关闭不会再被拉起。enable 失败不写
            // 标记，下次启动重试。debug 构建（dev/e2e）一律不碰：注册的会是
            // target/debug 下的 exe，开机拉起弹黑框且白屏。
            #[cfg(not(debug_assertions))]
            {
                let state = handle.state::<AppState>();
                let mut st = state.0.lock().unwrap();
                if st.config.autostart_defaulted.is_none() {
                    match handle.autolaunch().enable() {
                        Ok(()) => {
                            st.config.autostart_defaulted = Some(true);
                            if let Err(e) = st.config.save(&st.config_path) {
                                eprintln!("写入开机自启默认标记失败: {e}");
                            }
                            store::append_log_file(&store::LogEntry {
                                ts: chrono::Utc::now().timestamp_millis(),
                                provider_id: String::new(),
                                provider_name: String::new(),
                                kind: "schedule".into(),
                                ok: true,
                                detail: "已默认开启开机自启，重启后将自动后台运行".into(),
                            });
                        }
                        Err(e) => {
                            eprintln!("默认开启开机自启失败，下次启动重试: {e}");
                            store::append_log_file(&store::LogEntry {
                                ts: chrono::Utc::now().timestamp_millis(),
                                provider_id: String::new(),
                                provider_name: String::new(),
                                kind: "schedule".into(),
                                ok: false,
                                detail: format!("默认开启开机自启失败: {e}"),
                            });
                        }
                    }
                }
            }

            // 自启路径自愈（仅 release）：注册表里的路径可能指向旧位置——
            // 旧版本误注册的 target/debug、安装目录迁移等。只要自启处于开启
            // 状态，就用当前 exe 路径重写一次（enable 幂等，无副作用）；
            // 已关闭则不动，尊重用户选择。
            #[cfg(not(debug_assertions))]
            {
                let auto = handle.autolaunch();
                if auto.is_enabled().unwrap_or(false) {
                    if let Err(e) = auto.enable() {
                        eprintln!("校正开机自启路径失败: {e}");
                    }
                }
            }

            // 窗口默认隐藏（tauri.conf.json visible:false），手动打开（无
            // --minimized 参数）时显示；开机自启拉起则保持静默进托盘
            if !std::env::args().any(|a| a == "--minimized") {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }

            // 启动定时调度（每 30 秒检查一次触发时间）
            {
                let shared = handle.state::<AppState>().0.clone();
                scheduler::spawn_scheduler(handle.clone(), shared);
            }

            // 启动时预取一次支持的供应商额度
            {
                let handle2 = handle.clone();
                let shared = handle.state::<AppState>().0.clone();
                tauri::async_runtime::spawn(async move {
                    let providers: Vec<store::ProviderConfig> = {
                        let st = shared.lock().unwrap();
                        st.config.providers.clone()
                    };
                    for p in providers {
                        if quota::detect_kind(&p.base_url).is_some() {
                            commands::fetch_and_store_quota(handle2.clone(), shared.clone(), p)
                                .await;
                        }
                    }
                });
            }

            build_tray(&handle)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // 关闭窗口 = 隐藏到托盘，保持调度器运行
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
