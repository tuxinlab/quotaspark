//! Tauri 命令层：前端 invoke 的入口。

use crate::cc_sync::{self, SyncReport};
use crate::engine;
use crate::quota;
use crate::state::{log_and_emit, now_ms, AppState, Inner};
use crate::store::{LogEntry, ProviderConfig};
use chrono::NaiveTime;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use tauri_plugin_autostart::ManagerExt;

fn notify(app: &AppHandle) {
    let _ = app.emit("state-changed", ());
}

#[derive(Serialize)]
pub struct ProviderCard {
    #[serde(flatten)]
    pub provider: ProviderConfig,
    pub quota: Option<quota::QuotaResult>,
    /// 最近一次激活结果
    pub last: Option<LogEntry>,
    /// 是否支持自动查额度（前端据此禁用查额度入口，避免点了必报错）
    pub supports_quota: bool,
}

/// 进程内单调递增序号：同一次同步导入会在极短时间内连续生成多个 id，
/// Windows 时钟粒度粗于微秒，仅靠时间戳会在同一微秒内碰撞
/// （曾导致两个供应商共享同一 id，统计串显、删除连带）。
static ID_SEQ: AtomicU64 = AtomicU64::new(0);

fn new_id() -> String {
    let seq = ID_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("p_{}_{}", chrono::Utc::now().timestamp_micros(), seq)
}

fn opt_trim(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[tauri::command]
pub fn get_providers(state: tauri::State<AppState>) -> Vec<ProviderCard> {
    let st = state.0.lock().unwrap();
    st.config
        .providers
        .iter()
        .map(|p| {
            // 与 quota::dispatch 的路由一致：已知域名，或显式指定的智谱团队版
            let supports_quota = quota::detect_kind(&p.base_url).is_some()
                || p.plan_type
                    .as_deref()
                    .map(|s| s.eq_ignore_ascii_case("zhipu_team"))
                    .unwrap_or(false);
            ProviderCard {
                quota: st.quota.get(&p.id).cloned(),
                last: st
                    .logs
                    .iter()
                    .rev()
                    .find(|l| l.provider_id == p.id && l.kind == "activate")
                    .cloned(),
                provider: p.clone(),
                supports_quota,
            }
        })
        .collect()
}

#[tauri::command]
pub fn get_logs(state: tauri::State<AppState>) -> Vec<LogEntry> {
    let st = state.0.lock().unwrap();
    st.logs.iter().rev().take(200).cloned().collect()
}

/// 从 cc-switch 数据库导入/更新 Claude 供应商（按 cc provider id 去重）。
#[tauri::command]
pub async fn sync_from_cc(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<SyncReport, String> {
    let (list, skipped) = cc_sync::read_cc_providers()?;
    let shared = state.0.clone();
    let (report, detail) = {
        let mut st = shared.lock().unwrap();
        let mut imported = 0usize;
        let mut updated = 0usize;
        for c in list {
            if let Some(existing) = st
                .config
                .providers
                .iter_mut()
                .find(|p| p.cc_provider_id.as_deref() == Some(c.cc_id.as_str()))
            {
                existing.name = c.name;
                existing.base_url = c.base_url;
                existing.api_key = c.api_key;
                if !c.model.is_empty() {
                    existing.model = c.model;
                }
                // 额度查询凭据：cc-switch 里配了（非空）才覆盖，保留本地手动填的值
                if c.plan_type.is_some() {
                    existing.plan_type = c.plan_type;
                }
                if c.usage_url.is_some() {
                    existing.usage_url = c.usage_url;
                }
                if c.access_key_id.is_some() {
                    existing.access_key_id = c.access_key_id;
                }
                if c.secret_access_key.is_some() {
                    existing.secret_access_key = c.secret_access_key;
                }
                if c.team_organization_id.is_some() {
                    existing.team_organization_id = c.team_organization_id;
                }
                if c.team_project_id.is_some() {
                    existing.team_project_id = c.team_project_id;
                }
                updated += 1;
            } else {
                st.config.providers.push(ProviderConfig {
                    id: new_id(),
                    name: c.name,
                    base_url: c.base_url,
                    api_key: c.api_key,
                    model: c.model,
                    format: "anthropic".into(),
                    enabled: false,
                    times: Vec::new(),
                    cc_provider_id: Some(c.cc_id),
                    cc_app_type: Some("claude".into()),
                    usage_url: c.usage_url,
                    access_key_id: c.access_key_id,
                    secret_access_key: c.secret_access_key,
                    plan_type: c.plan_type,
                    team_organization_id: c.team_organization_id,
                    team_project_id: c.team_project_id,
                });
                imported += 1;
            }
        }
        let report = SyncReport {
            imported,
            updated,
            skipped,
            total: imported + updated + skipped,
        };
        let detail = format!(
            "导入 {imported} 个、更新 {updated} 个、跳过 {skipped} 个（官方登录或本地路由占位配置不含可用 Key）；额度查询凭据已按 cc-switch 配置同步",
        );
        let _ = st.config.save(&st.config_path);
        (report, detail)
    };
    log_and_emit(
        &app,
        &shared,
        LogEntry {
            ts: now_ms(),
            provider_id: String::new(),
            provider_name: "同步 cc-switch".into(),
            kind: "sync".into(),
            ok: true,
            detail,
        },
    );
    Ok(report)
}

#[tauri::command]
pub fn save_provider(
    app: AppHandle,
    state: tauri::State<AppState>,
    mut provider: ProviderConfig,
) -> Result<ProviderConfig, String> {
    if provider.name.trim().is_empty() {
        return Err("名称不能为空".into());
    }
    if provider.base_url.trim().is_empty() {
        return Err("Base URL 不能为空".into());
    }
    if provider.api_key.trim().is_empty() {
        return Err("API Key 不能为空".into());
    }
    if provider.model.trim().is_empty() {
        return Err("激活用模型不能为空".into());
    }
    provider.name = provider.name.trim().to_string();
    provider.base_url = provider.base_url.trim().to_string();
    provider.api_key = provider.api_key.trim().to_string();
    provider.model = provider.model.trim().to_string();
    provider.times = provider
        .times
        .iter()
        .map(|t| t.trim().to_string())
        .collect();
    for t in &provider.times {
        if NaiveTime::parse_from_str(t, "%H:%M").is_err() {
            return Err(format!("时间格式错误：{t}（应为 HH:MM）"));
        }
    }
    provider.times.sort();
    provider.times.dedup();
    provider.usage_url = opt_trim(provider.usage_url);
    provider.access_key_id = opt_trim(provider.access_key_id);
    provider.secret_access_key = opt_trim(provider.secret_access_key);
    provider.team_organization_id = opt_trim(provider.team_organization_id);
    provider.team_project_id = opt_trim(provider.team_project_id);
    provider.plan_type = match provider.plan_type.as_deref().map(str::trim) {
        Some(s) if s.eq_ignore_ascii_case("zhipu_team") => Some("zhipu_team".into()),
        _ => None,
    };

    let shared = state.0.clone();
    if provider.id.is_empty() {
        provider.id = new_id();
        {
            let mut st = shared.lock().unwrap();
            st.config.providers.push(provider.clone());
            st.config.save(&st.config_path)?;
        }
        log_and_emit(
            &app,
            &shared,
            LogEntry {
                ts: now_ms(),
                provider_id: provider.id.clone(),
                provider_name: provider.name.clone(),
                kind: "add".into(),
                ok: true,
                detail: format!("已添加供应商「{}」（{}）", provider.name, provider.base_url),
            },
        );
        return Ok(provider);
    }
    {
        let mut st = shared.lock().unwrap();
        match st.config.providers.iter_mut().find(|p| p.id == provider.id) {
            Some(p) => *p = provider.clone(),
            None => return Err("供应商不存在".into()),
        }
        st.config.save(&st.config_path)?;
    }
    notify(&app);
    Ok(provider)
}

#[tauri::command]
pub fn delete_provider(
    app: AppHandle,
    state: tauri::State<AppState>,
    id: String,
) -> Result<(), String> {
    let shared = state.0.clone();
    let (name, detail) = {
        let mut st = shared.lock().unwrap();
        let name = st
            .config
            .providers
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.clone())
            .ok_or_else(|| "供应商不存在".to_string())?;
        st.config.providers.retain(|p| p.id != id);
        st.quota.remove(&id);
        st.config.last_fired.remove(&id);
        st.config.last_activated.remove(&id);
        st.config.save(&st.config_path)?;
        let detail = format!("已删除供应商「{name}」（下次同步会按 cc-switch 现状重新导入）");
        (name, detail)
    };
    log_and_emit(
        &app,
        &shared,
        LogEntry {
            ts: now_ms(),
            provider_id: id,
            provider_name: name,
            kind: "delete".into(),
            ok: true,
            detail,
        },
    );
    Ok(())
}

/// 按前端给定的 id 顺序重排供应商（卡片拖拽排序）。config.json 的 providers
/// 顺序即展示顺序；ids 里没出现的（并发新增的）保持原有相对顺序追加在尾部。
/// 只调展示顺序，不写活动日志（拖拽不应刷屏）。
#[tauri::command]
pub fn reorder_providers(
    app: AppHandle,
    state: tauri::State<AppState>,
    ids: Vec<String>,
) -> Result<(), String> {
    let shared = state.0.clone();
    {
        let mut st = shared.lock().unwrap();
        let mut next: Vec<ProviderConfig> = Vec::with_capacity(st.config.providers.len());
        for id in &ids {
            if let Some(pos) = st.config.providers.iter().position(|p| &p.id == id) {
                next.push(st.config.providers.remove(pos));
            }
        }
        next.append(&mut st.config.providers);
        st.config.providers = next;
        st.config.save(&st.config_path)?;
    }
    notify(&app);
    Ok(())
}

#[tauri::command]
pub fn set_schedule(
    app: AppHandle,
    state: tauri::State<AppState>,
    id: String,
    enabled: bool,
    times: Vec<String>,
) -> Result<(), String> {
    let times: Vec<String> = times.iter().map(|t| t.trim().to_string()).collect();
    for t in &times {
        if NaiveTime::parse_from_str(t, "%H:%M").is_err() {
            return Err(format!("时间格式错误：{t}（应为 HH:MM）"));
        }
    }
    let mut st = state.0.lock().unwrap();
    let p = st
        .config
        .providers
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| "供应商不存在".to_string())?;
    p.enabled = enabled;
    p.times = times;
    st.config.save(&st.config_path)?;
    notify(&app);
    Ok(())
}

#[tauri::command]
pub fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    // debug 构建（npm run tauri dev / e2e）注册的会是 target/debug 下的 exe：
    // 开机拉起时带控制台黑框，且内嵌的是 Vite dev 地址（没有 dev 服务器时
    // 白屏报连接被拒）。e2e 会真实点击这个开关，必须在 dev 里挡掉写入，
    // 自启只在安装版（release）里允许开关。
    #[cfg(debug_assertions)]
    {
        let _ = (app, enabled);
        Err("开发调试模式不修改开机自启（避免把 debug 版路径注册进系统）".into())
    }
    #[cfg(not(debug_assertions))]
    {
        let auto = app.autolaunch();
        if enabled {
            auto.enable().map_err(|e| e.to_string())
        } else {
            auto.disable().map_err(|e| e.to_string())
        }
    }
}

/// 立即激活：传 id 激活单个，不传激活全部。
#[tauri::command]
pub async fn activate_now(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    id: Option<String>,
) -> Result<usize, String> {
    let shared = state.0.clone();
    match id {
        Some(id) => {
            let p = {
                let st = shared.lock().unwrap();
                st.config
                    .providers
                    .iter()
                    .find(|p| p.id == id)
                    .cloned()
                    .ok_or_else(|| "供应商不存在".to_string())?
            };
            spawn_activation(app, shared, p, false, 0);
            Ok(1)
        }
        None => {
            let n = { shared.lock().unwrap().config.providers.len() };
            activate_all_spawn(app, shared);
            Ok(n)
        }
    }
}

/// 供托盘菜单和命令共用的"全部激活"。
pub fn activate_all_spawn(app: AppHandle, shared: Arc<Mutex<Inner>>) {
    let providers: Vec<ProviderConfig> = {
        let st = shared.lock().unwrap();
        st.config.providers.clone()
    };
    for p in providers {
        spawn_activation(app.clone(), shared.clone(), p, false, 0);
    }
}

/// 发起一次激活。scheduled=true 为定时触发（激活后核对窗口是否真的点亮）；
/// delay_ms > 0 表示上一窗口未过期：先记录推迟日志，等窗口过期后再发
/// （推迟量由 scheduler::scheduled_delay_ms 算出）。手动激活传 false / 0。
pub fn spawn_activation(
    app: AppHandle,
    shared: Arc<Mutex<Inner>>,
    p: ProviderConfig,
    scheduled: bool,
    delay_ms: i64,
) {
    tauri::async_runtime::spawn(async move {
        if delay_ms > 0 {
            let secs = (delay_ms / 1000).max(1);
            log_and_emit(
                &app,
                &shared,
                LogEntry {
                    ts: now_ms(),
                    provider_id: p.id.clone(),
                    provider_name: p.name.clone(),
                    kind: "activate".into(),
                    ok: true,
                    detail: format!("上一激活窗口尚未过期，推迟 {secs} 秒再发，避免被旧窗口吞掉"),
                },
            );
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms as u64)).await;
        }
        let result = engine::activate(&p).await;
        let (ok, detail) = match result {
            Ok(d) => (true, d),
            Err(e) => (false, e),
        };
        // 记录成功激活时刻，供下一次定时触发判断上一窗口是否已过期
        let activated_at = now_ms();
        if ok {
            let mut st = shared.lock().unwrap();
            st.config.last_activated.insert(p.id.clone(), activated_at);
            let _ = st.config.save(&st.config_path);
        }
        log_and_emit(
            &app,
            &shared,
            LogEntry {
                ts: now_ms(),
                provider_id: p.id.clone(),
                provider_name: p.name.clone(),
                kind: "activate".into(),
                ok,
                detail: detail.clone(),
            },
        );
        // 激活成功后顺手刷新额度（如果该供应商支持），并核对窗口是否真的点亮
        if ok && quota::detect_kind(&p.base_url).is_some() {
            let q = fetch_and_store_quota(app.clone(), shared.clone(), p.clone()).await;
            let expected_reset = if scheduled {
                Some(activated_at + engine::WINDOW_MS)
            } else {
                None
            };
            if let Some((confirm_ok, confirm_detail)) = window_confirm(&q, expected_reset) {
                log_and_emit(
                    &app,
                    &shared,
                    LogEntry {
                        ts: now_ms(),
                        provider_id: p.id.clone(),
                        provider_name: p.name.clone(),
                        kind: "activate".into(),
                        ok: confirm_ok,
                        detail: confirm_detail,
                    },
                );
            }
        }
    });
}

#[tauri::command]
pub async fn query_quota(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let p = {
        let st = state.0.lock().unwrap();
        st.config
            .providers
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| "供应商不存在".to_string())?
    };
    fetch_and_store_quota(app, state.0.clone(), p).await;
    Ok(())
}

/// 查询额度并写入状态 + 日志，完成后发事件让前端刷新。
/// 返回查询结果，供激活后的窗口确认使用。
pub async fn fetch_and_store_quota(
    app: AppHandle,
    shared: Arc<Mutex<Inner>>,
    p: ProviderConfig,
) -> quota::QuotaResult {
    let q = quota::query_quota(&p).await;
    {
        let mut st = shared.lock().unwrap();
        st.quota.insert(p.id.clone(), q.clone());
    }
    let detail = if q.ok {
        let mut parts: Vec<String> = q
            .tiers
            .iter()
            .map(|t| {
                let mut s = format!("{} 已用 {:.0}%", t.name, t.utilization);
                if let Some(r) = &t.resets_at {
                    s.push_str(&format!("（{r} 重置）"));
                }
                s
            })
            .collect();
        if let Some(b) = &q.balance_text {
            parts.push(b.clone());
        }
        if parts.is_empty() {
            "查询成功".into()
        } else {
            let body = parts.join("；");
            match quota::detect_kind(&p.base_url).map(quota::detect_label) {
                Some(label) => format!("[{label}] {body}"),
                None => body,
            }
        }
    } else {
        q.error.clone().unwrap_or_else(|| "查询失败".into())
    };
    log_and_emit(
        &app,
        &shared,
        LogEntry {
            ts: now_ms(),
            provider_id: p.id.clone(),
            provider_name: p.name.clone(),
            kind: "quota".into(),
            ok: q.ok,
            detail,
        },
    );
    q
}

/// 窗口确认容差：覆盖 tick 抖动（数分钟内）、本机与供应商的时钟偏差、
/// 展示时间的分钟取整。
const CONFIRM_TOLERANCE_MS: i64 = 45 * 60 * 1000;

/// 激活成功后核对刚刷新的额度回包，确认五小时窗口是否真的点亮。
/// - 额度查询失败，或该供应商没有五小时窗口档位（按量余额类）→ None，不另记日志
/// - 有档位但无重置时间（如智谱 "0% 且无重置时间"）→ 未点亮
/// - 定时触发（expected_reset_ms = 本次激活 + 5h）：重置时间与预期偏差超过
///   容差 → 判定请求被仍在活动的旧窗口吸收，本次没有点燃新窗口
/// - 手动激活不比对预期，只要窗口点亮即视为成功
fn window_confirm(
    q: &quota::QuotaResult,
    expected_reset_ms: Option<i64>,
) -> Option<(bool, String)> {
    if !q.ok {
        return None;
    }
    let five = q.tiers.iter().find(|t| t.name == "五小时窗口")?;
    let Some(reset) = &five.resets_at else {
        return Some((
            false,
            "请求成功但五小时窗口未点亮（额度接口未返回活动窗口；若刚激活可能是统计延迟，可稍后刷新复核）"
                .into(),
        ));
    };
    let Some(expected) = expected_reset_ms else {
        return Some((true, format!("窗口点亮中（{reset} 重置）")));
    };
    let drift_ok = five
        .resets_at_ts
        .map(|ts| (ts * 1000 - expected).abs() <= CONFIRM_TOLERANCE_MS)
        .unwrap_or(false);
    if drift_ok {
        return Some((true, format!("已确认窗口点亮（{reset} 重置）")));
    }
    let exp_display = chrono::DateTime::from_timestamp_millis(expected)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    Some((
        false,
        format!(
            "激活请求疑似被仍在活动的旧窗口吸收：当前窗口 {reset} 重置，并非本次点燃（预期约 {exp_display} 重置）。若为接口统计延迟可稍后刷新复核"
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::WINDOW_MS;

    fn five_tier(resets_at: Option<&str>, ts: Option<i64>) -> quota::QuotaTier {
        quota::QuotaTier {
            name: "五小时窗口".into(),
            utilization: 1.0,
            resets_at: resets_at.map(str::to_string),
            resets_at_ts: ts,
            amount: None,
        }
    }

    fn q_ok(tiers: Vec<quota::QuotaTier>) -> quota::QuotaResult {
        quota::QuotaResult {
            ok: true,
            plan: None,
            tiers,
            balance_text: None,
            error: None,
            ts: 0,
        }
    }

    #[test]
    fn test_window_confirm() {
        let now = 1_800_000_000_000i64;
        let expected = now + WINDOW_MS;
        // 点亮且重置时间与预期吻合（差 1 分钟）→ 已确认
        let q = q_ok(vec![five_tier(
            Some("09-30 15:31"),
            Some((expected - 60_000) / 1000),
        )]);
        let (ok, detail) = window_confirm(&q, Some(expected)).unwrap();
        assert!(ok);
        assert!(detail.contains("已确认窗口点亮"));
        // 重置时间偏差 2 小时 → 判定被旧窗口吸收
        let q = q_ok(vec![five_tier(
            Some("09-30 13:31"),
            Some((expected - 7_200_000) / 1000),
        )]);
        let (ok, detail) = window_confirm(&q, Some(expected)).unwrap();
        assert!(!ok);
        assert!(detail.contains("吸收"));
        // 无重置时间 → 未点亮
        let (ok, detail) =
            window_confirm(&q_ok(vec![five_tier(None, None)]), Some(expected)).unwrap();
        assert!(!ok);
        assert!(detail.contains("未点亮"));
        // 手动激活：不比对预期，只要点亮即绿
        let q = q_ok(vec![five_tier(
            Some("09-30 13:31"),
            Some((expected - 7_200_000) / 1000),
        )]);
        let (ok, detail) = window_confirm(&q, None).unwrap();
        assert!(ok);
        assert!(detail.contains("窗口点亮中"));
        // 按量余额类（没有五小时窗口档位）与查询失败 → 不做确认
        assert!(window_confirm(&q_ok(vec![]), Some(expected)).is_none());
        assert!(window_confirm(&quota::QuotaResult::default(), Some(expected)).is_none());
    }

    /// 回归测试：同步导入循环会在极短时间内连续生成多个 id，
    /// 微秒时间戳在同一微秒内会碰撞，id 必须附加进程内单调序号保证唯一。
    #[test]
    fn rapid_new_id_calls_are_unique() {
        use std::collections::HashSet;

        const N: usize = 200_000;
        let ids: HashSet<String> = (0..N).map(|_| new_id()).collect();
        assert_eq!(ids.len(), N, "快速连续生成时 new_id 产生了重复 id");
    }
}
