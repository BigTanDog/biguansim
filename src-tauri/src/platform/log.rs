//! 极简文件日志：单文件滚动、最多 2 份、每份 ≤ 1 MB。
//!
//! 位置按 PRD §8 / F-10.10 取 `%APPDATA%\闭关模拟器\logs\biguansim.log`。
//! 注意：Tauri 的 `app_data_dir()` 会按 identifier 生成目录，与 PRD 指定的
//! `闭关模拟器` 目录不一致，所以这里读 `APPDATA` 环境变量再拼产品目录名，
//! 仍然不出现硬编码盘符（AGENTS §6）。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use log::{LevelFilter, Log, Metadata, Record};
use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

const APP_DIR_NAME: &str = "闭关模拟器";
const LOG_FILE: &str = "biguansim.log";
const MAX_BYTES: u64 = 1024 * 1024;
/// 除当前文件外额外保留的备份份数（log.1、log.2 由轮转产生）
const KEEP_BACKUPS: usize = 2;

struct FileLogger {
    dir: PathBuf,
    state: Mutex<State>,
}

struct State {
    file: Option<File>,
    bytes: u64,
}

impl FileLogger {
    fn rotate(&self, state: &mut State) {
        for i in (1..KEEP_BACKUPS).rev() {
            let from = self.dir.join(format!("{LOG_FILE}.{i}"));
            let to = self.dir.join(format!("{LOG_FILE}.{}", i + 1));
            if from.exists() {
                let _ = fs::rename(&from, &to);
            }
        }

        let current = self.dir.join(LOG_FILE);
        if current.exists() {
            let _ = fs::rename(&current, self.dir.join(format!("{LOG_FILE}.1")));
        }

        state.bytes = 0;
        state.file = open_append(&current);
    }
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let line = format!(
            "{} {:<5} {}: {}\n",
            timestamp(),
            record.level(),
            record.target(),
            record.args()
        );

        let Ok(mut state) = self.state.lock() else {
            return;
        };

        if state.bytes + line.len() as u64 > MAX_BYTES {
            self.rotate(&mut state);
        }

        let written = match state.file.as_mut() {
            Some(file) => file
                .write_all(line.as_bytes())
                .and_then(|()| file.flush())
                .is_ok(),
            None => false,
        };

        if written {
            state.bytes += line.len() as u64;
        }
    }

    fn flush(&self) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(file) = state.file.as_mut() {
                let _ = file.flush();
            }
        }
    }
}

/// 初始化全局 logger。重复调用或目录不可写时静默降级（日志不该阻断启动）。
pub fn init() {
    let dir = app_dir().join("logs");
    if fs::create_dir_all(&dir).is_err() {
        return;
    }

    let current = dir.join(LOG_FILE);
    let file = open_append(&current);
    let bytes = fs::metadata(&current).map(|m| m.len()).unwrap_or(0);

    let logger = FileLogger {
        dir,
        state: Mutex::new(State { file, bytes }),
    };

    if ::log::set_boxed_logger(Box::new(logger)).is_ok() {
        ::log::set_max_level(LevelFilter::Info);
    }
}

/// `%APPDATA%\闭关模拟器\`（PRD §8 的数据根目录）
pub fn app_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_DIR_NAME)
}

fn open_append(path: &PathBuf) -> Option<File> {
    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn timestamp() -> String {
    let mut st = SYSTEMTIME {
        wYear: 0,
        wMonth: 0,
        wDayOfWeek: 0,
        wDay: 0,
        wHour: 0,
        wMinute: 0,
        wSecond: 0,
        wMilliseconds: 0,
    };

    // SAFETY: GetLocalTime 只写入我们提供的栈上结构体，无别名与生命周期要求。
    unsafe { GetLocalTime(&mut st) };

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
    )
}
