pub mod platform;

use tauri::Manager;

pub fn run() {
    platform::log::init();
    ::log::info!("启动 biguansim {}", env!("CARGO_PKG_VERSION"));

    tauri::Builder::default()
        .setup(|app| {
            let Some(window) = app.get_webview_window("main") else {
                ::log::error!("主窗口不存在，无法继续");
                return Ok(());
            };

            // 窗口配置为 visible:false，避免"先小窗再全屏"的闪烁。
            // S5 会换成"按 rcMonitor 定位后再显示"，并保留 5 s 强制显示兜底；
            // 这里先直接显示，保证 S2 能看到窗口。
            window.show()?;
            ::log::info!("主窗口已显示");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri 运行时启动失败");
}
