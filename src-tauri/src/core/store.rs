//! 落盘：原子写、JSONL 追加、损坏行跳过、配置兜底（TDD §7 / PRD §8）。
//!
//! 时间戳一律 Unix 毫秒、字段名带 `*Ms` 后缀（`decisions/003`）。
//! 本模块不认识 `%APPDATA%`，根目录由调用方注入，便于单测。

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const CONFIG_FILE: &str = "config.json";
pub const RECORDS_FILE: &str = "records.jsonl";
pub const SESSION_FILE: &str = "session.json";
pub const CONFIG_VERSION: u32 = 1;

/// 一次闭关的结果（PRD F-9.3）
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionResult {
    Completed,
    Early,
    Emergency,
    Accident,
    Interrupted,
}

/// `records.jsonl` 的一行（PRD F-9.2）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
    pub planned_ms: i64,
    pub actual_ms: i64,
    pub result: SessionResult,
    pub attempts: u32,
    pub faults: u32,
    pub bank: String,
}

/// `config.json`（PRD §8）
///
/// `extra` 保留未知字段：用户可能手改过文件，版本升级也不该把不认识的键丢掉。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub version: u32,
    pub dim: bool,
    pub physical_brightness: bool,
    pub chime_volume: f32,
    pub cooldown_base: u32,
    pub question_base: u32,
    pub theme: String,
    pub last_duration_min: u32,
    pub recent_quotes: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            dim: true,
            physical_brightness: true,
            chime_volume: 0.6,
            cooldown_base: 5,
            question_base: 3,
            theme: "dark".to_string(),
            last_duration_min: 120,
            recent_quotes: Vec::new(),
            extra: Map::new(),
        }
    }
}

/// `session.json`：Locked 期间每 15 s 覆写，正常出关时删除
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub state: String,
    pub target_ms: i64,
    pub elapsed_ms: i64,
    pub mono_base: u64,
    pub wall_base_ms: i64,
    pub started_at_ms: i64,
    pub attempts: u32,
    pub cooldown_until_ms: Option<i64>,
    pub bank: String,
    pub goal: Option<String>,
}

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join(CONFIG_FILE)
    }

    pub fn records_path(&self) -> PathBuf {
        self.root.join(RECORDS_FILE)
    }

    pub fn session_path(&self) -> PathBuf {
        self.root.join(SESSION_FILE)
    }

    /// 原子写：同目录写 `.tmp` → fsync → rename 覆盖。
    /// rename 在 Windows 上走 `MOVEFILE_REPLACE_EXISTING`，可直接覆盖已有文件。
    pub fn atomic_write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let tmp = tmp_path(path);
        {
            let mut file = File::create(&tmp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }

        fs::rename(&tmp, path)
    }

    pub fn read_json<T: DeserializeOwned>(&self, path: &Path) -> std::io::Result<T> {
        let bytes = fs::read(path)?;
        serde_json::from_slice(&bytes)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
    }

    pub fn write_json<T: Serialize>(&self, path: &Path, value: &T) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(value)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        self.atomic_write(path, &bytes)
    }

    /// 追加一行 JSONL，写完立即 flush（崩溃最多丢当前行）
    pub fn append_jsonl<T: Serialize>(&self, path: &Path, value: &T) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut line = serde_json::to_string(value)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        line.push('\n');

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        file.write_all(line.as_bytes())?;
        file.flush()
    }

    /// 读全部流水。损坏行跳过并计数，供界面提示"部分记录损坏"。
    pub fn load_records(&self) -> std::io::Result<(Vec<Record>, usize)> {
        let path = self.records_path();
        if !path.exists() {
            return Ok((Vec::new(), 0));
        }

        let text = fs::read_to_string(&path)?;
        let mut records = Vec::new();
        let mut skipped = 0usize;

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<Record>(line) {
                Ok(record) => records.push(record),
                Err(_) => skipped += 1,
            }
        }

        Ok((records, skipped))
    }

    /// 读配置：文件不存在用默认值；解析失败则把坏文件改名备份为 `.bad` 再回默认值。
    pub fn load_config(&self) -> Config {
        let path = self.config_path();
        if !path.exists() {
            return Config::default();
        }

        match self.read_json::<Config>(&path) {
            Ok(config) => config,
            Err(_) => {
                let _ = fs::rename(&path, bad_path(&path));
                Config::default()
            }
        }
    }

    pub fn save_config(&self, config: &Config) -> std::io::Result<()> {
        self.write_json(&self.config_path(), config)
    }

    pub fn load_session(&self) -> Option<SessionSnapshot> {
        self.read_json::<SessionSnapshot>(&self.session_path()).ok()
    }

    pub fn save_session(&self, snapshot: &SessionSnapshot) -> std::io::Result<()> {
        self.write_json(&self.session_path(), snapshot)
    }

    pub fn clear_session(&self) -> std::io::Result<()> {
        let path = self.session_path();
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}

fn tmp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "data".to_string());
    path.with_file_name(format!("{name}.tmp"))
}

fn bad_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "data".to_string());
    path.with_file_name(format!("{name}.bad"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 测试用的临时目录，Drop 时删掉，避免在用户机器上堆垃圾
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!(
                "biguansim-test-{label}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("创建临时目录");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn sample_record(id: &str) -> Record {
        Record {
            id: id.to_string(),
            started_at_ms: 1_760_875_920_000,
            ended_at_ms: 1_760_883_120_000,
            planned_ms: 7_200_000,
            actual_ms: 7_200_000,
            result: SessionResult::Completed,
            attempts: 0,
            faults: 0,
            bank: "basic".to_string(),
        }
    }

    #[test]
    fn atomic_write_leaves_no_tmp_file() {
        let dir = TempDir::new("atomic");
        let store = Store::new(&dir.path);
        let path = store.config_path();

        store.atomic_write(&path, b"{\"a\":1}").expect("写入");

        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"a\":1}");
        assert!(!tmp_path(&path).exists(), "临时文件必须被 rename 消耗掉");
    }

    #[test]
    fn leftover_tmp_file_is_ignored_on_read() {
        let dir = TempDir::new("crash");
        let store = Store::new(&dir.path);
        store.save_config(&Config::default()).expect("写入配置");

        // 模拟"写完 tmp 但还没 rename 就断电"
        fs::write(tmp_path(&store.config_path()), b"{\"dim\":false}").unwrap();

        let config = store.load_config();
        assert!(config.dim, "读到的不该是残留 tmp 里的内容");
    }

    #[test]
    fn corrupt_jsonl_lines_are_skipped_and_counted() {
        let dir = TempDir::new("records");
        let store = Store::new(&dir.path);

        store
            .append_jsonl(&store.records_path(), &sample_record("s_1"))
            .unwrap();
        store
            .append_jsonl(&store.records_path(), &sample_record("s_2"))
            .unwrap();

        let mut text = fs::read_to_string(store.records_path()).unwrap();
        text.push_str("{\"id\":\"broken\"\n\n");
        text.push_str("{\"id\":\"s_3\",\"startedAtMs\":1,\"endedAtMs\":2,\"plannedMs\":3,\"actualMs\":4,\"result\":\"early\",\"attempts\":1,\"faults\":0,\"bank\":\"basic\"}\n");
        fs::write(store.records_path(), text).unwrap();

        let (records, skipped) = store.load_records().expect("读取流水");

        assert_eq!(records.len(), 3);
        assert_eq!(skipped, 1);
        assert_eq!(records[2].id, "s_3");
        assert_eq!(records[2].result, SessionResult::Early);
    }

    #[test]
    fn invalid_config_is_backed_up_and_defaults_returned() {
        let dir = TempDir::new("badcfg");
        let store = Store::new(&dir.path);
        fs::write(store.config_path(), b"{ not json").unwrap();

        let config = store.load_config();

        assert_eq!(config.version, CONFIG_VERSION);
        assert!(config.dim);
        assert!(
            bad_path(&store.config_path()).exists(),
            "坏文件应被备份为 .bad"
        );
        assert!(!store.config_path().exists(), "坏文件应已被移走");
    }

    #[test]
    fn unknown_config_fields_survive_round_trip() {
        let dir = TempDir::new("extra");
        let store = Store::new(&dir.path);
        fs::write(
            store.config_path(),
            br#"{"version":1,"dim":false,"futureFlag":42,"nested":{"a":1}}"#,
        )
        .unwrap();

        let config = store.load_config();
        assert!(!config.dim);
        assert_eq!(config.cooldown_base, 5, "缺失字段用默认值");
        store.save_config(&config).unwrap();

        let raw: Value =
            serde_json::from_str(&fs::read_to_string(store.config_path()).unwrap()).unwrap();
        assert_eq!(raw["futureFlag"], 42);
        assert_eq!(raw["nested"]["a"], 1);
    }
}
