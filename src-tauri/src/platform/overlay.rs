//! 全屏覆盖：主屏复用主窗口，副屏各开一个只读镜像窗口（TDD §4.3）。
//!
//! 几何一律取 `Monitor` 的完整区域（等价于 rcMonitor，含任务栏区），
//! 不用工作区，否则任务栏会露出来（PRD §13-8）。

use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

use crate::app_state::AppState;

/// 副屏窗口 label 前缀。前端靠 label 判断"镜像模式"，不依赖 URL 查询串——
/// dev 与生产两套 URL 形态不同，用 label 更稳。
pub const SECONDARY_PREFIX: &str = "display-";

pub const MAIN_LABEL: &str = "main";

/// Idle 态窗口尺寸（M1 用一个固定大小的配置页）
pub const IDLE_SIZE: (u32, u32) = (900, 640);

struct Rect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

pub fn lock_screen(app: &AppHandle) -> Result<(), String> {
    // 这里**不能**再用 `AppState.locked` 做重入保护：调用方为了保证置顶维持线程
    // 一启动就能看到 true，已经把标志置位了。入关的合法性由状态机（machine）负责。
    let monitors = app.available_monitors().map_err(|e| e.to_string())?;
    if monitors.is_empty() {
        return Err("没有检测到任何显示器".to_string());
    }

    let mut rects: Vec<Rect> = monitors
        .iter()
        .map(|m| Rect {
            x: m.position().x,
            y: m.position().y,
            width: m.size().width,
            height: m.size().height,
        })
        .collect();

    // 主屏排到第一个：优先用系统认定的 primary，拿不到就按枚举顺序
    if let Ok(Some(primary)) = app.primary_monitor() {
        let key = (primary.position().x, primary.position().y);
        if let Some(index) = rects.iter().position(|r| (r.x, r.y) == key) {
            rects.swap(0, index);
        }
    }

    let main = app
        .get_webview_window(MAIN_LABEL)
        .ok_or_else(|| "主窗口不存在".to_string())?;

    // 顺序很重要：必须先去掉装饰再定位定尺寸。
    // 带标题栏时 set_size 设的是含边框的外尺寸，去掉装饰后客户区会多出一圈，
    // 表现为"画面整体偏右下、边缘露出桌面"。
    main.set_decorations(false).map_err(|e| e.to_string())?;
    main.set_always_on_top(true).map_err(|e| e.to_string())?;
    place(&main, &rects[0])?;
    main.show().map_err(|e| e.to_string())?;
    // 起势窗口必须拿到焦点，否则 3 秒内的 Esc 取消收不到按键（F-2.2）
    let _ = main.set_focus();

    for (index, rect) in rects.iter().enumerate().skip(1) {
        let label = format!("{SECONDARY_PREFIX}{index}");
        let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
            .title("闭关模拟器")
            .decorations(false)
            .resizable(false)
            .focused(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .build()
            .map_err(|e| format!("副屏窗口创建失败：{e}"))?;

        place(&window, rect)?;
        window.show().map_err(|e| e.to_string())?;
    }

    // `AppState.locked` 的 true 只由 commands::start_session 在调用本函数**之前**置位，
    // 否则下面这个 keeper 线程会先看到 false 然后立刻退出（掷硬币，实测会随机失效）。
    log::info!("结界已展开，共 {} 块屏", rects.len());

    spawn_topmost_keeper(app.clone());
    Ok(())
}

pub fn unlock_screen(app: &AppHandle) {
    // 幂等清理：调用方忘了置 false 也不会让窗口永远置顶
    app.state::<AppState>().set_locked(false);

    for (label, window) in app.webview_windows() {
        if label.starts_with(SECONDARY_PREFIX) {
            let _ = window.close();
        }
    }

    if let Some(main) = app.get_webview_window(MAIN_LABEL) {
        let _ = main.set_always_on_top(false);
        let _ = main.set_decorations(true);
        let _ = main.set_size(PhysicalSize::new(IDLE_SIZE.0, IDLE_SIZE.1));
        let _ = main.center();
        let _ = main.set_focus();
    }

    log::info!("结界已收起");
}

/// 任务栏本身也是 topmost，只在创建时置顶会被压回去；
/// Locked 期间每 1 s 重申一次（TDD §4.3）。
fn spawn_topmost_keeper(app: AppHandle) {
    thread::spawn(move || {
        while app.state::<AppState>().is_locked() {
            for (label, window) in app.webview_windows() {
                if label == MAIN_LABEL || label.starts_with(SECONDARY_PREFIX) {
                    let _ = window.set_always_on_top(true);
                }
            }
            thread::sleep(Duration::from_secs(1));
        }
    });
}

fn place(window: &tauri::WebviewWindow, rect: &Rect) -> Result<(), String> {
    window
        .set_position(PhysicalPosition::new(rect.x, rect.y))
        .map_err(|e| e.to_string())?;
    window
        .set_size(PhysicalSize::new(rect.width, rect.height))
        .map_err(|e| e.to_string())?;
    Ok(())
}
