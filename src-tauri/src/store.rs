//! 配置与日志的持久化：~/.cc-activator/config.json + logs/activator.log

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

fn default_format() -> String {
    "anthropic".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub model: String,
    /// "anthropic" | "openai"
    #[serde(default = "default_format")]
    pub format: String,
    /// 定时激活总开关
    #[serde(default)]
    pub enabled: bool,
    /// 每日定时激活时间，"HH:MM"
    #[serde(default)]
    pub times: Vec<String>,
    /// 来源 cc-switch 的 provider id（同步去重用），手动添加为 None
    #[serde(default)]
    pub cc_provider_id: Option<String>,
    #[serde(default)]
    pub cc_app_type: Option<String>,
    /// 额度查询高级配置：ZenMux 用量端点 URL
    #[serde(default)]
    pub usage_url: Option<String>,
    /// 火山方舟用量查询 AccessKey ID（与推理 Key 是两套凭据）
    #[serde(default)]
    pub access_key_id: Option<String>,
    #[serde(default)]
    pub secret_access_key: Option<String>,
    /// 无法靠 base_url 区分时显式指定，目前仅 "zhipu_team"
    #[serde(default)]
    pub plan_type: Option<String>,
    /// 智谱团队版组织 ID / 项目 ID
    #[serde(default)]
    pub team_organization_id: Option<String>,
    #[serde(default)]
    pub team_project_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    /// 供应商 id -> 最近一次定时触发的去重键 "id|日期|HH:MM"
    #[serde(default)]
    pub last_fired: HashMap<String, String>,
    /// 供应商 id -> 最近一次成功激活的时刻（毫秒时间戳）。
    /// 定时激活据此判断上一窗口（+5h）是否仍未过期，见 scheduler::scheduled_delay_ms
    #[serde(default)]
    pub last_activated: HashMap<String, i64>,
    /// 开机自启默认开启的一次性标记。缺失 = 尚未引导（旧配置/全新安装），
    /// 启动时会自动补开自启并写标记；标记存在后即以顶栏开关的注册状态为准。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autostart_defaulted: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogEntry {
    pub ts: i64,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub provider_name: String,
    /// activate | quota | sync | schedule
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub detail: String,
}

pub fn data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cc-activator")
}

impl Config {
    pub fn load(path: &Path) -> Config {
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// 原子写入：先写临时文件再改名，避免写坏配置。
    /// path 是软链接时（dotfiles 用户，移植自 cc-switch #6764/#9283）写透到链接
    /// 指向的文件，链接本身保留——rename 到链接本身会把链接替换成普通文件。
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let path = &resolve_symlink_target(path);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&tmp, text).map_err(|e| e.to_string())?;
        fs::rename(&tmp, path).map_err(|e| e.to_string())
    }
}

/// `path` 是软链接时返回它最终指向的文件（逐跳 `read_link`，相对目标按链接所在
/// 目录解析），否则原样返回。原子写要替换的是这个最终文件。只解析最后一段，
/// 不碰路径里的目录软链；除相对目标带 `..` 外不做 `canonicalize`（Windows 上会
/// 把路径改写成 `\\?\` 形式）。悬空链接照样返回它指向的路径，写入时会在那里
/// 新建文件、链接保留。（移植自 cc-switch #6764/#9283）
fn resolve_symlink_target(path: &Path) -> PathBuf {
    // 与 Linux 的 MAXSYMLINKS 一致，防止链接成环时死循环。
    const MAX_HOPS: usize = 40;
    let mut current = path.to_path_buf();
    for _ in 0..MAX_HOPS {
        let is_link = fs::symlink_metadata(&current)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false);
        if !is_link {
            return current;
        }
        let Ok(target) = fs::read_link(&current) else {
            return current;
        };
        current = if target.is_absolute() {
            target
        } else {
            match current.parent() {
                Some(parent) => join_link_target(parent, &target),
                None => target,
            }
        };
    }
    // 层数过多视为异常，按原路径写入（链接保留与否不再保证，至少不写坏数据）。
    path.to_path_buf()
}

/// 把相对链接目标接到链接所在目录上。目标里有 `..` 时先把目录解析成真实路径再
/// 逐段回退：Windows 按字面折叠 `..`，链接所在目录本身是 junction / 目录软链时
/// 会退到错误的上级目录（Unix 内核按真实路径解析，结果一样）。
fn join_link_target(parent: &Path, target: &Path) -> PathBuf {
    if !target
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return parent.join(target);
    }
    let Ok(mut resolved) = fs::canonicalize(parent) else {
        return parent.join(target);
    };
    for component in target.components() {
        match component {
            Component::ParentDir => {
                // 已走过的段里也可能有目录软链，回退前先解析成真实路径。
                if let Ok(real) = fs::canonicalize(&resolved) {
                    resolved = real;
                }
                resolved.pop();
            }
            Component::CurDir => {}
            other => resolved.push(other.as_os_str()),
        }
    }
    resolved
}

pub fn append_log_file(entry: &LogEntry) {
    let dir = data_dir().join("logs");
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let line = serde_json::json!({
        "ts": entry.ts,
        "provider_id": entry.provider_id,
        "provider_name": entry.provider_name,
        "kind": entry.kind,
        "ok": entry.ok,
        "detail": entry.detail,
    });
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("activator.log"))
    {
        let _ = writeln!(f, "{line}");
    }
}

/// 启动时读回最近 n 条日志（文件按时间追加，读尾部后反转回时间正序）。
pub fn load_log_tail(n: usize) -> Vec<LogEntry> {
    let path = data_dir().join("logs").join("activator.log");
    let Ok(content) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut out: Vec<LogEntry> = content
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str(l).ok())
        .take(n)
        .collect();
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_path_resolves_to_itself() {
        // 存在的普通文件 / 不存在的路径：都原样返回
        let mut tmp = std::env::temp_dir();
        tmp.push(format!("qs-plain-{}", std::process::id()));
        fs::write(&tmp, b"x").unwrap();
        assert_eq!(resolve_symlink_target(&tmp), tmp);
        fs::remove_file(&tmp).unwrap();
        assert_eq!(
            resolve_symlink_target(Path::new("/no/such/config.json")),
            PathBuf::from("/no/such/config.json")
        );
    }

    /// dotfiles 用户把 config.json 软链到别的目录（#6764/#9283）：原子写要写透到
    /// 真实文件、链接保留、临时文件不留残留。两跳相对链接：home/config.json ->
    /// home/hop.json -> ../dotfiles/config.json。Windows 创建符号链接需要开发者
    /// 模式或管理员权限，无权限时跳过。
    #[test]
    fn save_writes_through_symlink_and_keeps_it() {
        let base = std::env::temp_dir().join(format!(
            "qs-symlink-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dotfiles = base.join("dotfiles");
        let home = base.join("home");
        fs::create_dir_all(&dotfiles).unwrap();
        fs::create_dir_all(&home).unwrap();
        let real = dotfiles.join("config.json");
        fs::write(&real, "old").unwrap();

        #[cfg(unix)]
        let mk = |src: &Path, dst: &Path| std::os::unix::fs::symlink(src, dst);
        #[cfg(windows)]
        let mk = |src: &Path, dst: &Path| std::os::windows::fs::symlink_file(src, dst);
        let hop = home.join("hop.json");
        let link = home.join("config.json");
        if mk(&real, &hop).is_err() || mk(Path::new("hop.json"), &link).is_err() {
            let _ = fs::remove_dir_all(&base);
            return; // 无符号链接权限，跳过
        }

        Config::default().save(&link).unwrap();

        // 链接本身保留（没有被替换成普通文件）
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "软链接被替换成了普通文件"
        );
        // 写透到链接指向的真实文件
        let saved = fs::read_to_string(&real).unwrap();
        assert!(
            saved.contains("providers") && saved != "old",
            "真实文件未更新"
        );
        // 经链接读回一致
        assert_eq!(fs::read_to_string(&link).unwrap(), saved);
        // 临时文件写在真实文件旁，无残留
        assert!(!dotfiles.join("config.json.tmp").exists());
        assert!(!home.join("config.json.tmp").exists());

        let _ = fs::remove_dir_all(&base);
    }
}
