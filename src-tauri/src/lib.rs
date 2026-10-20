pub mod app_state;
pub mod commands;
pub mod core;
pub mod events;
pub mod platform;
pub mod runtime;

use std::time::Duration;

use tauri::{Manager, WindowEvent};

pub fn run() {
    platform::log::init();
    ::log::info!("启动 biguansim {}", env!("CARGO_PKG_VERSION"));

    tauri::Builder::default()
        .manage(app_state::AppState::default())
        // F-1.7：第二次启动只唤前台；锁定期间第二实例什么都不做
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            let locked = app
                .try_state::<app_state::AppState>()
                .map(|state| state.is_locked())
                .unwrap_or(false);

            if locked {
                ::log::info!("锁定期间忽略第二次启动");
                return;
            }

            if let Some(window) = app.get_webview_window(platform::overlay::MAIN_LABEL) {
                let _ = window.show();
                let _ = window.set_focus();
                ::log::info!("第二次启动：已唤到前台");
            }
        }))
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::start_session,
            commands::cancel_arming,
            commands::get_state
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let runtime = runtime::init(&handle);
            app.manage(runtime);

            let Some(window) = app.get_webview_window(platform::overlay::MAIN_LABEL) else {
                ::log::error!("主窗口不存在，无法继续");
                return Ok(());
            };

            // visible:false 是为了避免"先小窗再全屏"的闪烁；万一定位逻辑出错，
            // 5 秒后强制显示，否则用户看到的是"双击没反应"。
            let fallback = window.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(5));
                if !fallback.is_visible().unwrap_or(false) {
                    ::log::warn!("窗口 5 秒内未显示，强制显示兜底");
                    let _ = fallback.show();
                }
            });

            window.show()?;
            ::log::info!("主窗口已显示");
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let locked = window
                    .app_handle()
                    .try_state::<app_state::AppState>()
                    .map(|state| state.is_locked())
                    .unwrap_or(false);

                if locked {
                    api.prevent_close();
                    ::log::warn!("锁定期间拒绝关闭窗口");
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("Tauri 运行时启动失败");
}
