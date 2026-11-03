//! 额度 / 余额查询：全部端点与解析逻辑移植自 cc-switch（MIT License）。
//!
//! 按 base_url 自动检测：
//! - Coding Plan（窗口百分比）：Kimi For Coding、智谱 GLM 个人版（bigmodel.cn /
//!   api.z.ai）、智谱团队版（需显式指定 + 组织/项目 ID）、MiniMax（中/英）、
//!   ZenMux（需填用量端点 URL）、OpenCode Go、Command Code、
//!   火山方舟 Agent/Coding Plan（需账号 AK/SK）
//! - 按量余额：DeepSeek、StepFun、SiliconFlow（中/英）、OpenRouter、Novita AI
//!
//! 智谱个人版账户没有 Coding Plan（按量付费）时查套餐会回业务错误，此时改查
//! 账户余额展示；团队版不回落（余额接口不认组织/项目头）。
//!
//! 官方 OAuth 订阅（Claude / ChatGPT / Gemini / Copilot / xAI）依赖 cc-switch 的
//! OAuth token 刷新链路，本工具不搬移。

use crate::engine::{client, truncate};
use crate::store::ProviderConfig;
use serde::{Deserialize, Serialize};
use std::time::Duration;

// ── 类型 ────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuotaTier {
    pub name: String,
    /// 已用百分比 0-100（火山 AFP 可能超 100，展示层负责裁剪）
    pub utilization: f64,
    /// 本地时区重置时间展示串，如 "09-28 15:00"
    pub resets_at: Option<String>,
    /// 重置时间 epoch 秒（与 resets_at 同源；激活后的窗口确认用它做时间比对）
    #[serde(default)]
    pub resets_at_ts: Option<i64>,
    /// 金额补充说明，如 "$1.20 / $5.00"
    #[serde(default)]
    pub amount: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QuotaResult {
    pub ok: bool,
    pub plan: Option<String>,
    pub tiers: Vec<QuotaTier>,
    /// 按量余额类结果的展示串
    #[serde(default)]
    pub balance_text: Option<String>,
    pub error: Option<String>,
    pub ts: i64,
}

// ── 供应商识别 ──────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Kimi,
    Zhipu,
    MiniMaxCn,
    MiniMaxEn,
    ZenMux,
    OpencodeGo,
    CommandCode,
    Volcengine,
    DeepSeek,
    StepFun,
    SiliconFlowCn,
    SiliconFlowEn,
    OpenRouter,
    Novita,
}

pub fn detect_kind(base_url: &str) -> Option<Kind> {
    let u = base_url.to_lowercase();
    // Coding Plan 优先
    if u.contains("api.kimi.com/coding") {
        Some(Kind::Kimi)
    } else if u.contains("bigmodel.cn") || u.contains("api.z.ai") {
        Some(Kind::Zhipu)
    } else if u.contains("api.minimaxi.com") || u.contains("api.minimax.cn") {
        Some(Kind::MiniMaxCn)
    } else if u.contains("api.minimax.io") {
        Some(Kind::MiniMaxEn)
    } else if u.contains("opencode.ai/zen/go") {
        Some(Kind::OpencodeGo)
    } else if u.contains("api.commandcode.ai/provider") {
        Some(Kind::CommandCode)
    } else if u.contains("volces.com/api/plan") || u.contains("volces.com/api/coding") {
        Some(Kind::Volcengine)
    } else if u.contains("zenmux") {
        Some(Kind::ZenMux)
    }
    // 按量余额
    else if u.contains("api.deepseek.com") {
        Some(Kind::DeepSeek)
    } else if u.contains("api.stepfun.ai") || u.contains("api.stepfun.com") {
        Some(Kind::StepFun)
    } else if u.contains("api.siliconflow.cn") {
        Some(Kind::SiliconFlowCn)
    } else if u.contains("api.siliconflow.com") {
        Some(Kind::SiliconFlowEn)
    } else if u.contains("openrouter.ai") {
        Some(Kind::OpenRouter)
    } else if u.contains("api.novita.ai") {
        Some(Kind::Novita)
    } else {
        None
    }
}

pub fn detect_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Kimi => "Kimi For Coding",
        Kind::Zhipu => "智谱 GLM Coding Plan",
        Kind::MiniMaxCn | Kind::MiniMaxEn => "MiniMax Coding Plan",
        Kind::ZenMux => "ZenMux",
        Kind::OpencodeGo => "OpenCode Go",
        Kind::CommandCode => "Command Code",
        Kind::Volcengine => "火山方舟 Agent/Coding Plan",
        Kind::DeepSeek => "DeepSeek（按量）",
        Kind::StepFun => "StepFun（按量）",
        Kind::SiliconFlowCn | Kind::SiliconFlowEn => "SiliconFlow（按量）",
        Kind::OpenRouter => "OpenRouter（按量）",
        Kind::Novita => "Novita AI（按量）",
    }
}

// ── 入口 ────────────────────────────────────────────────────

pub async fn query_quota(p: &ProviderConfig) -> QuotaResult {
    let result = dispatch(p).await;
    match result {
        Ok(mut q) => {
            q.ts = now_ms();
            q
        }
        Err(e) => QuotaResult {
            ok: false,
            error: Some(e),
            ts: now_ms(),
            ..Default::default()
        },
    }
}

async fn dispatch(p: &ProviderConfig) -> Result<QuotaResult, String> {
    // 智谱团队版：base_url 与个人版完全相同，无法自动区分，必须显式指定
    if p.plan_type
        .as_deref()
        .map(|s| s.eq_ignore_ascii_case("zhipu_team"))
        .unwrap_or(false)
    {
        return zhipu_team(p).await;
    }
    let Some(kind) = detect_kind(&p.base_url) else {
        return Err("未识别的供应商域名，暂不支持自动查额度（激活不受影响）".into());
    };
    match kind {
        Kind::Kimi => kimi(p).await,
        Kind::Zhipu => zhipu(p).await,
        Kind::MiniMaxCn => minimax(p, true).await,
        Kind::MiniMaxEn => minimax(p, false).await,
        Kind::ZenMux => zenmux(p).await,
        Kind::OpencodeGo => opencode_go(p).await,
        Kind::CommandCode => command_code(p).await,
        Kind::Volcengine => volcengine(p).await,
        Kind::DeepSeek => deepseek(p).await,
        Kind::StepFun => stepfun(p).await,
        Kind::SiliconFlowCn => siliconflow(p, true).await,
        Kind::SiliconFlowEn => siliconflow(p, false).await,
        Kind::OpenRouter => openrouter(p).await,
        Kind::Novita => novita(p).await,
    }
}

// ── 通用辅助 ────────────────────────────────────────────────

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

async fn http_get(
    url: &str,
    headers: &[(&'static str, String)],
) -> Result<(reqwest::StatusCode, String), String> {
    let mut req = client().get(url).timeout(Duration::from_secs(15));
    for (k, v) in headers {
        req = req.header(*k, v.as_str());
    }
    let resp = req.send().await.map_err(|e| format!("请求失败：{e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    Ok((status, text))
}

fn require_key(p: &ProviderConfig) -> Result<String, String> {
    let k = p.api_key.trim();
    if k.is_empty() {
        return Err("API Key 不能为空".into());
    }
    Ok(k.to_string())
}

fn parse_json(text: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(text).map_err(|e| format!("解析失败：{e}"))
}

fn auth_failed(status: reqwest::StatusCode) -> String {
    format!("Key 无效或已过期（HTTP {status}）")
}

fn num(v: &serde_json::Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn num_field(obj: &serde_json::Value, field: &str) -> Option<f64> {
    obj.get(field).and_then(num)
}

/// 重置时间统一转本地时区展示。兼容：秒/毫秒时间戳（数字或字符串）、ISO 8601 字符串。
fn reset_display(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => {
            let s = s.trim();
            if let Ok(n) = s.parse::<i64>() {
                ts_to_local(n)
            } else {
                iso_to_local(s)
            }
        }
        serde_json::Value::Number(_) => v.as_i64().and_then(ts_to_local),
        _ => None,
    }
}

fn ts_to_local(n: i64) -> Option<String> {
    if n <= 0 {
        return None;
    }
    let ms = if n < 1_000_000_000_000 { n * 1000 } else { n };
    chrono::DateTime::from_timestamp_millis(ms).map(|d| {
        d.with_timezone(&chrono::Local)
            .format("%m-%d %H:%M")
            .to_string()
    })
}

fn iso_to_local(s: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| {
        d.with_timezone(&chrono::Local)
            .format("%m-%d %H:%M")
            .to_string()
    })
}

/// 重置时间统一转 epoch 秒（与 reset_display 同源），供窗口点亮确认做时间比对。
fn reset_ts(v: &serde_json::Value) -> Option<i64> {
    match v {
        serde_json::Value::String(s) => {
            let s = s.trim();
            if let Ok(n) = s.parse::<i64>() {
                norm_ts(n)
            } else {
                chrono::DateTime::parse_from_rfc3339(s)
                    .ok()
                    .map(|d| d.timestamp())
            }
        }
        serde_json::Value::Number(_) => v.as_i64().and_then(norm_ts),
        _ => None,
    }
}

fn norm_ts(n: i64) -> Option<i64> {
    if n <= 0 {
        return None;
    }
    let secs = if n < 1_000_000_000_000 { n } else { n / 1000 };
    // 早于 2001-09 的值视为异常数据
    if secs < 1_000_000_000 {
        return None;
    }
    Some(secs)
}

// ── Kimi For Coding ─────────────────────────────────────────
// GET https://api.kimi.com/coding/v1/usages（Bearer）

async fn kimi(p: &ProviderConfig) -> Result<QuotaResult, String> {
    let key = require_key(p)?;
    let (status, text) = http_get(
        "https://api.kimi.com/coding/v1/usages",
        &[
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ],
    )
    .await?;
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(auth_failed(status));
    }
    if !status.is_success() {
        return Err(format!("HTTP {status}：{}", truncate(&text, 200)));
    }
    let body = parse_json(&text)?;

    let mut tiers = Vec::new();
    if let Some(limits) = body.get("limits").and_then(|v| v.as_array()) {
        for item in limits {
            if let Some(d) = item.get("detail") {
                let limit = d.get("limit").and_then(num).unwrap_or(1.0);
                let remaining = d.get("remaining").and_then(num).unwrap_or(0.0);
                let util = if limit > 0.0 {
                    ((limit - remaining).max(0.0) / limit) * 100.0
                } else {
                    0.0
                };
                tiers.push(QuotaTier {
                    name: "五小时窗口".into(),
                    utilization: util,
                    resets_at: d.get("resetTime").and_then(reset_display),
                    resets_at_ts: d.get("resetTime").and_then(reset_ts),
                    amount: None,
                });
            }
        }
    }
    if let Some(u) = body.get("usage") {
        let limit = u.get("limit").and_then(num).unwrap_or(1.0);
        let remaining = u.get("remaining").and_then(num).unwrap_or(0.0);
        let util = if limit > 0.0 {
            ((limit - remaining).max(0.0) / limit) * 100.0
        } else {
            0.0
        };
        tiers.push(QuotaTier {
            name: "本周额度".into(),
            utilization: util,
            resets_at: u.get("resetTime").and_then(reset_display),
            resets_at_ts: u.get("resetTime").and_then(reset_ts),
            amount: None,
        });
    }
    if tiers.is_empty() {
        return Err("响应中没有额度数据".into());
    }
    Ok(QuotaResult {
        ok: true,
        plan: None,
        tiers,
        balance_text: None,
        error: None,
        ts: 0,
    })
}

// ── 智谱 GLM（个人版 / 团队版共用解析）──────────────────────
// GET {open.bigmodel.cn | api.z.ai}/api/monitor/usage/quota/limit
// 注意：Authorization 直接放 Key，不带 Bearer 前缀。unit=3 是五小时窗口，unit=6 是周窗口。

fn zhipu_tiers_from_body(
    body: &serde_json::Value,
) -> Result<(Vec<QuotaTier>, Option<String>), String> {
    if body.get("success").and_then(|v| v.as_bool()) == Some(false) {
        return Err(format!(
            "接口错误：{}",
            body.get("msg").and_then(|v| v.as_str()).unwrap_or("未知")
        ));
    }
    let data = body
        .get("data")
        .ok_or_else(|| "响应缺少 data 字段".to_string())?;
    let plan = data
        .get("level")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut five: Option<(f64, Option<String>, Option<i64>)> = None;
    let mut weekly: Option<(f64, Option<String>, Option<i64>)> = None;
    let mut other: Vec<(f64, Option<String>, Option<i64>)> = Vec::new();
    if let Some(limits) = data.get("limits").and_then(|v| v.as_array()) {
        for item in limits {
            let t = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if !t.eq_ignore_ascii_case("TOKENS_LIMIT") && !t.eq_ignore_ascii_case("CREDIT_LIMIT") {
                continue;
            }
            let util = item
                .get("percentage")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let entry = (
                util,
                item.get("nextResetTime").and_then(reset_display),
                item.get("nextResetTime").and_then(reset_ts),
            );
            match item.get("unit").and_then(|v| v.as_i64()) {
                Some(3) => {
                    if five.is_none() {
                        five = Some(entry);
                    }
                }
                Some(6) => {
                    if weekly.is_none() {
                        weekly = Some(entry);
                    }
                }
                _ => other.push(entry),
            }
        }
    }
    // unit 缺失时的兜底：第一条归五小时，第二条归周（智谱最多两条）
    let mut others = other.into_iter();
    if five.is_none() {
        five = others.next();
    }
    if weekly.is_none() {
        weekly = others.next();
    }

    let mut tiers = Vec::new();
    if let Some((util, reset, ts)) = five {
        tiers.push(QuotaTier {
            name: "五小时窗口".into(),
            utilization: util,
            resets_at: reset,
            resets_at_ts: ts,
            amount: None,
        });
    }
    if let Some((util, reset, ts)) = weekly {
        tiers.push(QuotaTier {
            name: "本周额度".into(),
            utilization: util,
            resets_at: reset,
            resets_at_ts: ts,
            amount: None,
        });
    }
    if tiers.is_empty() {
        return Err("响应中没有额度数据".into());
    }
    Ok((tiers, plan))
}

fn zhipu_host(base_url: &str) -> &'static str {
    if base_url.to_lowercase().contains("bigmodel.cn") {
        "https://open.bigmodel.cn"
    } else {
        "https://api.z.ai"
    }
}

async fn zhipu(p: &ProviderConfig) -> Result<QuotaResult, String> {
    let key = require_key(p)?;
    let url = format!("{}/api/monitor/usage/quota/limit", zhipu_host(&p.base_url));
    let (status, text) = http_get(
        &url,
        &[
            ("Authorization", key),
            ("Accept-Language", "en-US,en".into()),
        ],
    )
    .await?;
    if !status.is_success() {
        return Err(format!("HTTP {status}：{}", truncate(&text, 200)));
    }
    let body = parse_json(&text)?;
    // 按量付费账户查套餐时接口回 HTTP 200 + 业务错误「当前用户不存在coding plan」：
    // 改查账户余额，其他业务错误照常报（移植自 cc-switch #8465）
    if zhipu_body_has_no_plan(&body) {
        return zhipu_balance(p).await;
    }
    let (tiers, plan) = zhipu_tiers_from_body(&body)?;
    Ok(QuotaResult {
        ok: true,
        plan,
        tiers,
        balance_text: None,
        error: None,
        ts: 0,
    })
}

/// 智谱团队版：同一 quota 路径加 ?type=2，额外携带
/// bigmodel-organization / bigmodel-project 头（三者缺一不可）。
async fn zhipu_team(p: &ProviderConfig) -> Result<QuotaResult, String> {
    let key = require_key(p)?;
    let org = p.team_organization_id.as_deref().unwrap_or("").trim();
    let proj = p.team_project_id.as_deref().unwrap_or("").trim();
    if org.is_empty() || proj.is_empty() {
        return Err("智谱团队版需要在编辑表单的高级配置里填写组织 ID 与项目 ID".into());
    }
    let (status, text) = http_get(
        "https://open.bigmodel.cn/api/monitor/usage/quota/limit?type=2",
        &[
            ("Authorization", key),
            ("bigmodel-organization", org.to_string()),
            ("bigmodel-project", proj.to_string()),
            ("Accept-Language", "en-US,en".into()),
        ],
    )
    .await?;
    if !status.is_success() {
        return Err(format!("HTTP {status}：{}", truncate(&text, 200)));
    }
    let body = parse_json(&text)?;
    let (tiers, plan) = zhipu_tiers_from_body(&body)?;
    Ok(QuotaResult {
        ok: true,
        plan,
        tiers,
        balance_text: None,
        error: None,
        ts: 0,
    })
}

// ── 智谱 GLM 余额（个人版无 Coding Plan 时回落）─────────────
// GET {open.bigmodel.cn | api.z.ai}/api/biz/account/query-customer-account-report
// 没有公开文档。cc-switch 2026-10-18 用国内站按量 Key 实测：接受推理用的 API Key
//（带不带 Bearer 都行；同在 /api/biz/ 下的其他接口回 403，这个是有意开放的），
// Response: { code, msg, success, data: { balance, availableBalance, frozenBalance, ... } }，
// 单位人民币。国际站路由存在（无效 Key 回 401 而非 404），返回结构与币种未实测：
// 按同一结构解析，币种记 USD。团队版不回落：余额接口不认组织/项目头，
// 查到的未必是这个团队的钱。
async fn zhipu_balance(p: &ProviderConfig) -> Result<QuotaResult, String> {
    let key = require_key(p)?;
    let is_cn = p.base_url.to_lowercase().contains("bigmodel.cn");
    let url = format!(
        "{}/api/biz/account/query-customer-account-report",
        zhipu_host(&p.base_url)
    );
    let (status, text) = http_get(
        &url,
        &[
            ("Authorization", key), // 与套餐接口一致，不加 Bearer
            ("Accept", "application/json".into()),
            ("Accept-Language", "en-US,en".into()),
        ],
    )
    .await?;
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(auth_failed(status));
    }
    if !status.is_success() {
        return Err(format!("HTTP {status}：{}", truncate(&text, 200)));
    }
    let body = parse_json(&text)?;
    parse_zhipu_balance(&body, is_cn).map(|balance_text| QuotaResult {
        ok: true,
        plan: None,
        tiers: vec![],
        balance_text: Some(balance_text),
        error: None,
        ts: 0,
    })
}

/// availableBalance 是扣掉冻结后的可用余额；没有时退回 balance。余额为 0 不代表
/// Key 失效，照常显示 0（同 OpenRouter 的口径，cc-switch #8175）。
fn parse_zhipu_balance(body: &serde_json::Value, is_cn: bool) -> Result<String, String> {
    if body.get("success").and_then(|v| v.as_bool()) == Some(false) {
        return Err(format!(
            "接口错误：{}",
            body.get("msg").and_then(|v| v.as_str()).unwrap_or("未知")
        ));
    }
    let data = body.get("data").filter(|d| d.is_object()).unwrap_or(body);
    let available = num_field(data, "availableBalance")
        .or_else(|| num_field(data, "balance"))
        .filter(|v| v.is_finite())
        .ok_or_else(|| "响应缺少或含非法的余额字段".to_string())?;
    let unit = if is_cn { "CNY" } else { "USD" };
    let label = if is_cn { "智谱 GLM" } else { "Z.ai" };
    Ok(format!("{label} 余额：{available:.2} {unit}"))
}

/// 按量账户查套餐时，接口回 HTTP 200 + 业务错误（cc-switch 2026-10-18 国内站实测）：
/// `{"code":500,"msg":"当前用户不存在coding plan","success":false}`。
/// 只认「msg 提到 coding plan 且说没有」，Key 无效（HTTP 401/403）、网络错误、
/// 其他业务错误都不算，免得把真故障显示成余额。英文站文案未实测，关键词放宽了几个说法。
fn zhipu_body_has_no_plan(body: &serde_json::Value) -> bool {
    if body.get("success").and_then(|v| v.as_bool()) != Some(false) {
        return false;
    }
    let msg = body
        .get("msg")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    let compact: String = msg
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .collect();
    compact.contains("codingplan")
        && [
            "不存在",
            "没有",
            "未开通",
            "not exist",
            "not found",
            "not have",
            "doesn't have",
            "no coding plan",
            "not subscribed",
        ]
        .iter()
        .any(|kw| msg.contains(kw))
}

// ── MiniMax ─────────────────────────────────────────────────
// GET https://{api.minimaxi.com | api.minimax.io}/v1/api/openplatform/coding_plan/remains
// 新接口直接给"剩余百分比"，反转为已用百分比；只取 model_name == "general" 的条目。

async fn minimax(p: &ProviderConfig, is_cn: bool) -> Result<QuotaResult, String> {
    let key = require_key(p)?;
    let domain = if is_cn {
        "api.minimaxi.com"
    } else {
        "api.minimax.io"
    };
    let url = format!("https://{domain}/v1/api/openplatform/coding_plan/remains");
    let (status, text) = http_get(&url, &[("Authorization", format!("Bearer {key}"))]).await?;
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(auth_failed(status));
    }
    if !status.is_success() {
        return Err(format!("HTTP {status}：{}", truncate(&text, 200)));
    }
    let body = parse_json(&text)?;
    if let Some(br) = body.get("base_resp") {
        let code = br.get("status_code").and_then(|v| v.as_i64()).unwrap_or(-1);
        if code != 0 {
            return Err(format!(
                "接口错误（code {code}）：{}",
                br.get("status_msg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("未知")
            ));
        }
    }
    let item = body
        .get("model_remains")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|i| i.get("model_name").and_then(|v| v.as_str()) == Some("general"))
        })
        .ok_or_else(|| "响应中没有 general 模型的额度数据".to_string())?;

    let mut tiers = Vec::new();
    if let Some(remain) = item
        .get("current_interval_remaining_percent")
        .and_then(|v| v.as_f64())
    {
        tiers.push(QuotaTier {
            name: "五小时窗口".into(),
            utilization: 100.0 - remain,
            resets_at: item.get("end_time").and_then(reset_display),
            resets_at_ts: item.get("end_time").and_then(reset_ts),
            amount: None,
        });
    }
    // 仅当 status=1 时有周限额；status=3 等表示该套餐无周限额
    if item.get("current_weekly_status").and_then(|v| v.as_i64()) == Some(1) {
        if let Some(remain) = item
            .get("current_weekly_remaining_percent")
            .and_then(|v| v.as_f64())
        {
            tiers.push(QuotaTier {
                name: "本周额度".into(),
                utilization: 100.0 - remain,
                resets_at: item.get("weekly_end_time").and_then(reset_display),
                resets_at_ts: item.get("weekly_end_time").and_then(reset_ts),
                amount: None,
            });
        }
    }
    if tiers.is_empty() {
        return Err("响应中没有额度数据".into());
    }
    Ok(QuotaResult {
        ok: true,
        plan: None,
        tiers,
        balance_text: None,
        error: None,
        ts: 0,
    })
}

// ── ZenMux ──────────────────────────────────────────────────
// GET {用量端点 URL，用户在高级配置里填}（Bearer）
// data.quota_5_hour / quota_7_day：usage_percentage 为 0-1 小数，附 USD 金额。

async fn zenmux(p: &ProviderConfig) -> Result<QuotaResult, String> {
    let key = require_key(p)?;
    let url = p
        .usage_url
        .as_deref()
        .unwrap_or("")
        .trim()
        .trim_end_matches('/')
        .to_string();
    if url.is_empty() {
        return Err("ZenMux 需要在编辑表单的高级配置里填写用量端点 URL".into());
    }
    let (status, text) = http_get(
        &url,
        &[
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ],
    )
    .await?;
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(auth_failed(status));
    }
    if !status.is_success() {
        return Err(format!("HTTP {status}：{}", truncate(&text, 200)));
    }
    let body = parse_json(&text)?;
    if body.get("success").and_then(|v| v.as_bool()) != Some(true) {
        return Err(format!(
            "接口错误：{}",
            body.get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("未知")
        ));
    }
    let data = body
        .get("data")
        .ok_or_else(|| "响应缺少 data 字段".to_string())?;

    let mut tiers = Vec::new();
    for (field, name) in [("quota_5_hour", "五小时窗口"), ("quota_7_day", "本周额度")] {
        let Some(q) = data.get(field) else { continue };
        let util = q.get("usage_percentage").and_then(num).unwrap_or(0.0) * 100.0;
        let used = q.get("used_value_usd").and_then(num);
        let max = q.get("max_value_usd").and_then(num);
        let amount = match (used, max) {
            (Some(u), Some(m)) => Some(format!("${u:.2} / ${m:.2}")),
            (Some(u), None) => Some(format!("已用 ${u:.2}")),
            _ => None,
        };
        tiers.push(QuotaTier {
            name: name.into(),
            utilization: util,
            resets_at: q
                .get("resets_at")
                .and_then(|v| v.as_str())
                .and_then(iso_to_local),
            resets_at_ts: q.get("resets_at").and_then(reset_ts),
            amount,
        });
    }
    if tiers.is_empty() {
        return Err("响应中没有额度数据".into());
    }
    let plan = data
        .get("plan")
        .and_then(|pl| pl.get("tier"))
        .and_then(|v| v.as_str())
        .map(|s| format!("ZenMux {s}"));
    Ok(QuotaResult {
        ok: true,
        plan,
        tiers,
        balance_text: None,
        error: None,
        ts: 0,
    })
}

// ── OpenCode Go ─────────────────────────────────────────────
// GET https://opencode.ai/zen/go/v1/usage（Bearer）
// usage.{rolling|weekly|monthly}：{status, percent(0-100 已用), resetsAt}

async fn opencode_go(p: &ProviderConfig) -> Result<QuotaResult, String> {
    let key = require_key(p)?;
    let (status, text) = http_get(
        "https://opencode.ai/zen/go/v1/usage",
        &[
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ],
    )
    .await?;
    if status == reqwest::StatusCode::FORBIDDEN {
        return Err("Key 有效但该账号没有 OpenCode Go 订阅（HTTP 403）".into());
    }
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(auth_failed(status));
    }
    if !status.is_success() {
        return Err(format!("HTTP {status}：{}", truncate(&text, 200)));
    }
    let body = parse_json(&text)?;
    let usage = body
        .get("usage")
        .ok_or_else(|| "响应缺少 usage 字段".to_string())?;

    let mut tiers = Vec::new();
    for (field, name) in [
        ("rolling", "五小时窗口"),
        ("weekly", "本周额度"),
        ("monthly", "本月额度"),
    ] {
        let Some(w) = usage.get(field) else { continue };
        let Some(pct) = w.get("percent").and_then(num) else {
            continue;
        };
        // percent 为 0 时上游的 resetsAt 是占位值，丢弃不展示
        let (resets_at, resets_at_ts) = if pct > 0.0 {
            (
                w.get("resetsAt").and_then(reset_display),
                w.get("resetsAt").and_then(reset_ts),
            )
        } else {
            (None, None)
        };
        tiers.push(QuotaTier {
            name: name.into(),
            utilization: pct,
            resets_at,
            resets_at_ts,
            amount: None,
        });
    }
    if tiers.is_empty() {
        return Err("响应形态不认识（未文档化端点可能已变更）".into());
    }
    Ok(QuotaResult {
        ok: true,
        plan: Some("OpenCode Go".into()),
        tiers,
        balance_text: None,
        error: None,
        ts: 0,
    })
}

// ── Command Code ────────────────────────────────────────────
// 额度与窗口接口由官方 CLI 使用、未公开文档化（移植自 cc-switch）。
// `/alpha` 控制面固定在根域名，推理数据面才是 `/provider`(`/provider/v1`)，
// 因此查询不复用 base_url。流程：whoami 取 orgId → credits / subscriptions，
// usage/summary 以订阅期起点为 since 取 totalCost；5h/周滚动窗口仅在
// windowLimits.limited=true 时返回，月度池按 已花/（已花+剩余credits） 计。

const COMMAND_CODE_API_BASE: &str = "https://api.commandcode.ai";

fn command_code_url(path: &str, params: &[(&str, &str)]) -> Result<String, String> {
    let mut url = reqwest::Url::parse(&format!("{COMMAND_CODE_API_BASE}{path}"))
        .map_err(|e| format!("Command Code 端点非法：{e}"))?;
    {
        let mut query = url.query_pairs_mut();
        for (k, v) in params {
            query.append_pair(k, v);
        }
    }
    Ok(url.into())
}

async fn command_code_get(
    path: &str,
    params: &[(&str, &str)],
    key: &str,
) -> Result<serde_json::Value, String> {
    let url = command_code_url(path, params)?;
    let (status, text) = http_get(
        &url,
        &[
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ],
    )
    .await?;
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(auth_failed(status));
    }
    if !status.is_success() {
        return Err(format!("HTTP {status}：{}", truncate(&text, 200)));
    }
    parse_json(&text)
}

fn command_code_window_tier(window: Option<&serde_json::Value>, name: &str) -> Option<QuotaTier> {
    let window = window?;
    let used = window.get("used").and_then(num)?;
    let cap = window.get("cap").and_then(num)?;
    if cap <= 0.0 {
        return None;
    }
    Some(QuotaTier {
        name: name.into(),
        utilization: used / cap * 100.0,
        resets_at: window.get("resetAt").and_then(reset_display),
        resets_at_ts: window.get("resetAt").and_then(reset_ts),
        amount: None,
    })
}

fn parse_command_code_quota(
    credits_response: &serde_json::Value,
    subscription_response: Option<&serde_json::Value>,
    summary_response: &serde_json::Value,
) -> Result<QuotaResult, String> {
    let credit_root = credits_response
        .get("credits")
        .filter(|v| !v.is_null())
        .ok_or_else(|| "响应缺少 credits 字段（未文档化端点可能已变更）".to_string())?;

    // 月度 tier 是套餐级月度额度：purchased/free credits 只参与月度百分比，
    // 不参与 5h/周滚动窗口（与 OpenCode Go 一致，展示三类窗口 + 重置时间）
    let monthly_remaining = credit_root
        .get("monthlyCredits")
        .and_then(num)
        .unwrap_or(0.0)
        .max(0.0);
    let purchased_remaining = credit_root
        .get("purchasedCredits")
        .and_then(num)
        .unwrap_or(0.0)
        .max(0.0);
    let free_remaining = credit_root
        .get("freeCredits")
        .and_then(num)
        .unwrap_or(0.0)
        .max(0.0);
    let total_remaining = monthly_remaining + purchased_remaining + free_remaining;
    let total_spent = summary_response
        .get("totalCost")
        .and_then(num)
        .ok_or_else(|| "响应缺少 totalCost 字段（未文档化端点可能已变更）".to_string())?
        .max(0.0);
    let total_pool = total_spent + total_remaining;

    let subscription = subscription_response.and_then(|b| b.get("data"));
    let period_end = subscription.and_then(|d| d.get("currentPeriodEnd"));
    let plan = subscription
        .and_then(|d| d.get("planId"))
        .and_then(|v| v.as_str())
        .or_else(|| credit_root.get("planId").and_then(|v| v.as_str()))
        .map(str::to_string);

    let mut tiers = Vec::new();
    // 实际响应把 windowLimits 放在 credits 同级；保留嵌套位置兼容已出现过的变体
    if let Some(window_limits) = credits_response
        .get("windowLimits")
        .or_else(|| credit_root.get("windowLimits"))
    {
        if window_limits
            .get("limited")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            tiers.extend(command_code_window_tier(
                window_limits.get("fiveHour"),
                "五小时窗口",
            ));
            tiers.extend(command_code_window_tier(
                window_limits.get("weekly"),
                "本周额度",
            ));
        }
    }
    tiers.push(QuotaTier {
        name: "本月额度".into(),
        utilization: if total_pool > 0.0 {
            total_spent / total_pool * 100.0
        } else {
            0.0
        },
        resets_at: period_end.and_then(reset_display),
        resets_at_ts: period_end.and_then(reset_ts),
        amount: None,
    });

    Ok(QuotaResult {
        ok: true,
        plan,
        tiers,
        balance_text: None,
        error: None,
        ts: 0,
    })
}

async fn command_code(p: &ProviderConfig) -> Result<QuotaResult, String> {
    let key = require_key(p)?;
    let whoami = command_code_get("/alpha/whoami", &[("limits", "1")], &key).await?;
    let org_id = whoami
        .get("org")
        .and_then(|org| org.get("id"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let org_params: Vec<(&str, &str)> = org_id
        .as_deref()
        .map(|id| vec![("orgId", id)])
        .unwrap_or_default();

    let credits = command_code_get("/alpha/billing/credits", &org_params, &key).await?;
    let subscription = command_code_get("/alpha/billing/subscriptions", &org_params, &key).await?;
    let period_start = subscription
        .get("data")
        .and_then(|d| d.get("currentPeriodStart"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let mut summary_params: Vec<(&str, &str)> = Vec::new();
    if let Some(since) = period_start.as_deref() {
        summary_params.push(("since", since));
    }
    if let Some(id) = org_id.as_deref() {
        summary_params.push(("orgId", id));
    }
    let summary = command_code_get("/alpha/usage/summary", &summary_params, &key).await?;
    parse_command_code_quota(&credits, Some(&subscription), &summary)
}

// ── 火山方舟 Agent Plan / Coding Plan ───────────────────────
//
// 控制面 OpenAPI（open.volcengineapi.com），强制火山签名 V4（AK/SK）——
// 推理 Bearer Key 会被网关 400 拒绝。算法是 AWS SigV4 的火山变体（移植自
// cc-switch，对照官方 volc-openapi-demos）：
//   1. canonical headers 与 SignedHeaders 用固定顺序 host;x-date;x-content-sha256;content-type
//      （不按字母序）；
//   2. algorithm 串 `HMAC-SHA256`（无 AWS4 前缀）、credential scope 结尾 `request`
//      （非 aws4_request）、签名密钥 kDate=HMAC(SK, date)（SK 不加 AWS4 前缀）。
// 自动探测：按 base_url 数据面端点定顺序（/api/coding 先 Coding，其余先 Agent），
// 先查的未订阅/出错仍回落到另一个。不能固定先查 Agent：AK/SK 按账号发放，
// 同一账号可同时订阅两种套餐，固定顺序会把 Coding Plan 卡片显示成 Agent 额度。

const VOLC_OPENAPI_HOST: &str = "open.volcengineapi.com";
const VOLC_API_VERSION: &str = "2024-01-01";
const VOLC_DEFAULT_REGION: &str = "cn-beijing";
const VOLC_SERVICE: &str = "ark";
const VOLC_CONTENT_TYPE: &str = "application/json; charset=utf-8";
const VOLC_SIGNED_HEADERS: &str = "host;x-date;x-content-sha256;content-type";
const VOLC_AKSK_HINT: &str =
    "请检查 AccessKey ID / Secret 是否正确、账号是否有方舟用量查询（OpenAPI）权限。";

fn volc_hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn volc_sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(data))
}

/// RFC3986 unreserved 之外全部按 %XX 编码
fn volc_uri_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// 从数据面 base_url 提取 Region（ark.cn-beijing.volces.com → cn-beijing）
fn volc_region(base_url: &str) -> String {
    let host = base_url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(base_url)
        .split('/')
        .next()
        .unwrap_or("");
    host.split('.')
        .find(|p| p.starts_with("cn-") || p.starts_with("ap-"))
        .map(|p| p.to_string())
        .unwrap_or_else(|| VOLC_DEFAULT_REGION.to_string())
}

fn volc_canonical_query(action: &str, region: &str) -> String {
    let mut pairs = [
        ("Action", action),
        ("Region", region),
        ("Version", VOLC_API_VERSION),
    ];
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", volc_uri_encode(k), volc_uri_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// 生成 (Authorization, X-Date, X-Content-Sha256)，三者都要塞进请求头。
fn volc_sign(
    access_key_id: &str,
    secret_access_key: &str,
    region: &str,
    canonical_query: &str,
    body: &[u8],
    now: chrono::DateTime<chrono::Utc>,
) -> (String, String, String) {
    let x_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let short_date = now.format("%Y%m%d").to_string();
    let x_content_sha256 = volc_sha256_hex(body);

    // 火山特有：canonical headers 固定顺序，不按字母序
    let canonical_headers = format!(
        "host:{VOLC_OPENAPI_HOST}\nx-date:{x_date}\nx-content-sha256:{x_content_sha256}\ncontent-type:{VOLC_CONTENT_TYPE}\n"
    );
    let canonical_request = format!(
        "POST\n/\n{canonical_query}\n{canonical_headers}\n{VOLC_SIGNED_HEADERS}\n{x_content_sha256}"
    );

    let credential_scope = format!("{short_date}/{region}/{VOLC_SERVICE}/request");
    let string_to_sign = format!(
        "HMAC-SHA256\n{x_date}\n{credential_scope}\n{}",
        volc_sha256_hex(canonical_request.as_bytes())
    );

    let k_date = volc_hmac(secret_access_key.as_bytes(), short_date.as_bytes());
    let k_region = volc_hmac(&k_date, region.as_bytes());
    let k_service = volc_hmac(&k_region, VOLC_SERVICE.as_bytes());
    let k_signing = volc_hmac(&k_service, b"request");
    let signature: String = volc_hmac(&k_signing, string_to_sign.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let authorization = format!(
        "HMAC-SHA256 Credential={access_key_id}/{credential_scope}, SignedHeaders={VOLC_SIGNED_HEADERS}, Signature={signature}"
    );
    (authorization, x_date, x_content_sha256)
}

fn volc_is_auth_error_code(code: &str) -> bool {
    let c = code.to_lowercase();
    c.contains("auth")
        || c.contains("signature")
        || c.contains("accessdenied")
        || c.contains("denied")
        || c.contains("unauthorized")
        || c.contains("forbidden")
        || c.contains("credential")
        || c.contains("token")
}

fn volc_response_error(body: &serde_json::Value) -> Option<(String, String)> {
    let err = body
        .get("ResponseMetadata")
        .and_then(|m| m.get("Error"))
        .or_else(|| body.get("Error"))?;
    let code = err
        .get("Code")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let msg = err
        .get("Message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if code.is_empty() && msg.is_empty() {
        None
    } else {
        Some((code, msg))
    }
}

/// 单次 OpenAPI 调用的归类结果：鉴权失败（两个 plan 共用 AK/SK，命中即停）
/// 与其他失败分开，便于探测逻辑决定是否继续试另一个 plan。
enum VolcCall {
    Body(serde_json::Value),
    Auth(String),
    Other(String),
}

async fn volc_openapi_call(
    region: &str,
    access_key_id: &str,
    secret_access_key: &str,
    action: &str,
) -> VolcCall {
    let canonical_query = volc_canonical_query(action, region);
    let url = format!("https://{VOLC_OPENAPI_HOST}/?{canonical_query}");
    let body: &[u8] = b"";
    let (authorization, x_date, x_content_sha256) = volc_sign(
        access_key_id,
        secret_access_key,
        region,
        &canonical_query,
        body,
        chrono::Utc::now(),
    );

    let resp = client()
        .post(&url)
        .header("X-Date", x_date)
        .header("X-Content-Sha256", x_content_sha256)
        .header("Content-Type", VOLC_CONTENT_TYPE)
        .header("Authorization", authorization)
        .body(body.to_vec())
        .timeout(Duration::from_secs(15))
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return VolcCall::Other(format!("网络错误：{e}")),
    };

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return VolcCall::Auth(format!("鉴权失败（HTTP {status}）。{VOLC_AKSK_HINT}"));
    }
    let raw = match resp.text().await {
        Ok(t) => t,
        Err(e) => return VolcCall::Other(format!("读取响应失败：{e}")),
    };
    if !status.is_success() {
        // 火山网关对签名/凭据类错误常返 4xx 并携带 ResponseMetadata.Error 信封
        if let Ok(body) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some((code, msg)) = volc_response_error(&body) {
                if volc_is_auth_error_code(&code) {
                    return VolcCall::Auth(format!(
                        "鉴权失败（HTTP {status}，{code}）：{msg}。{VOLC_AKSK_HINT}"
                    ));
                }
                return VolcCall::Other(format!("接口错误（HTTP {status}，{code}）：{msg}"));
            }
        }
        return VolcCall::Other(format!(
            "接口错误（HTTP {status}）：{}",
            truncate(&raw, 200)
        ));
    }
    let body = match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(v) => v,
        Err(_) => return VolcCall::Other("响应不是合法 JSON".into()),
    };
    if let Some((code, msg)) = volc_response_error(&body) {
        if volc_is_auth_error_code(&code) {
            return VolcCall::Auth(format!("鉴权失败（{code}）：{msg}。{VOLC_AKSK_HINT}"));
        }
        return VolcCall::Other(format!("接口错误（{code}）：{msg}"));
    }
    VolcCall::Body(body)
}

/// GetAFPUsage：展示 5h / 周 / 月三个窗口；Quota<=0 视为未订阅该窗口。
fn parse_afp_tiers(result: &serde_json::Value) -> Vec<QuotaTier> {
    let mut tiers = Vec::new();
    for (key, name) in [
        ("AFPFiveHour", "五小时窗口"),
        ("AFPWeekly", "本周额度"),
        ("AFPMonthly", "本月额度"),
    ] {
        let Some(win) = result.get(key) else { continue };
        let quota = win.get("Quota").and_then(num).unwrap_or(0.0);
        if quota <= 0.0 {
            continue;
        }
        let used = win.get("Used").and_then(num).unwrap_or(0.0);
        tiers.push(QuotaTier {
            name: name.into(),
            utilization: used / quota * 100.0,
            resets_at: win.get("ResetTime").and_then(reset_display),
            resets_at_ts: win.get("ResetTime").and_then(reset_ts),
            amount: Some(format!("{used:.2} / {quota:.2}")),
        });
    }
    tiers
}

/// GetCodingPlanUsage 的窗口标签归一（真实字段 Level：session/weekly/monthly）
fn volc_window(label: &str) -> Option<&'static str> {
    match label.to_lowercase().as_str() {
        "session" | "5h" | "fivehour" | "five_hour" | "rolling_5h" => Some("五小时窗口"),
        "weekly" | "week" | "7d" => Some("本周额度"),
        "monthly" | "month" => Some("本月额度"),
        _ => None,
    }
}

/// GetCodingPlanUsage：宽松匹配数组与字段名，只给百分比，重置时间是秒级。
fn parse_coding_plan_tiers(result: &serde_json::Value) -> Vec<QuotaTier> {
    let mut tiers = Vec::new();
    let arr = result
        .get("QuotaUsage")
        .and_then(|v| v.as_array())
        .or_else(|| result.get("Usages").and_then(|v| v.as_array()))
        .or_else(|| result.get("Details").and_then(|v| v.as_array()));
    let Some(arr) = arr else { return tiers };

    for item in arr {
        let label = item
            .get("Level")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("Type").and_then(|v| v.as_str()))
            .or_else(|| item.get("Period").and_then(|v| v.as_str()))
            .or_else(|| item.get("Label").and_then(|v| v.as_str()))
            .or_else(|| item.get("Window").and_then(|v| v.as_str()))
            .unwrap_or("");
        let Some(name) = volc_window(label) else {
            continue;
        };
        let utilization = item
            .get("Percent")
            .and_then(num)
            .or_else(|| item.get("UsedPercent").and_then(num))
            .or_else(|| item.get("UsagePercent").and_then(num))
            .unwrap_or(0.0);
        let reset_raw = item.get("ResetTime").or_else(|| item.get("ResetTimestamp"));
        tiers.push(QuotaTier {
            name: name.into(),
            utilization,
            resets_at: reset_raw.and_then(reset_display),
            resets_at_ts: reset_raw.and_then(reset_ts),
            amount: None,
        });
    }
    tiers
}

/// 两种套餐的用量查询走不同的 OpenAPI Action
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VolcPlan {
    Afp,
    Coding,
}

impl VolcPlan {
    fn action(&self) -> &'static str {
        match self {
            VolcPlan::Afp => "GetAFPUsage",
            VolcPlan::Coding => "GetCodingPlanUsage",
        }
    }
}

/// 探测顺序：base_url 的数据面端点即用户意图（/api/coding=Coding，/api/plan=Agent），
/// 未识别的端点保持旧行为先查 Agent。返回（先查，回落）。
fn volc_probe_order(base_url: &str) -> (VolcPlan, VolcPlan) {
    if base_url.contains("/api/coding") {
        (VolcPlan::Coding, VolcPlan::Afp)
    } else {
        (VolcPlan::Afp, VolcPlan::Coding)
    }
}

/// 单个套餐探测结果
enum VolcProbe {
    Found(QuotaResult),
    /// 签名通过但没有可解析的额度数据（多半未订阅），带原始响应片段
    Empty(String),
    /// 网络或接口错误
    Failed(String),
    /// 鉴权失败，终止：AK/SK 是账号级凭据，另一个接口也必然失败
    Auth(String),
}

async fn volc_probe(region: &str, ak: &str, sk: &str, plan: VolcPlan) -> VolcProbe {
    let body = match volc_openapi_call(region, ak, sk, plan.action()).await {
        VolcCall::Auth(detail) => return VolcProbe::Auth(detail),
        VolcCall::Other(detail) => return VolcProbe::Failed(detail),
        VolcCall::Body(body) => body,
    };
    let result = body.get("Result").unwrap_or(&body);
    let (tiers, plan_label) = match plan {
        VolcPlan::Afp => {
            let tiers = parse_afp_tiers(result);
            let label = result
                .get("PlanType")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| format!("Agent Plan {s}"));
            (tiers, label)
        }
        VolcPlan::Coding => (parse_coding_plan_tiers(result), Some("Coding Plan".into())),
    };
    if tiers.is_empty() {
        return VolcProbe::Empty(truncate(&body.to_string(), 400));
    }
    VolcProbe::Found(QuotaResult {
        ok: true,
        plan: plan_label,
        tiers,
        balance_text: None,
        error: None,
        ts: 0,
    })
}

async fn volcengine(p: &ProviderConfig) -> Result<QuotaResult, String> {
    let ak = p.access_key_id.as_deref().unwrap_or("").trim();
    let sk = p.secret_access_key.as_deref().unwrap_or("").trim();
    if ak.is_empty() || sk.is_empty() {
        return Err(
            "火山方舟用量查询需要账号的 AccessKey ID + Secret（与推理 API Key 是两套凭据），请在编辑表单的高级配置里填写"
                .into(),
        );
    }
    let region = volc_region(&p.base_url);
    let (primary, fallback) = volc_probe_order(&p.base_url);

    let mut soft_errors: Vec<String> = Vec::new();
    let mut empty_responses: Vec<String> = Vec::new();
    for plan in [primary, fallback] {
        match volc_probe(&region, ak, sk, plan).await {
            VolcProbe::Found(result) => return Ok(result),
            VolcProbe::Auth(detail) => return Err(detail),
            VolcProbe::Failed(detail) => soft_errors.push(format!("{}：{detail}", plan.action())),
            VolcProbe::Empty(raw) => empty_responses.push(format!("{}={}", plan.action(), raw)),
        }
    }

    if !soft_errors.is_empty() {
        Err(soft_errors.join("；"))
    } else if !empty_responses.is_empty() {
        Err(format!(
            "签名已通过但没有可解析的额度数据（可能未订阅）。原始响应：{}",
            empty_responses.join(" || ")
        ))
    } else {
        Err("该凭据下没有找到有效的 Agent Plan 或 Coding Plan 订阅".into())
    }
}

// ── 按量余额 ────────────────────────────────────────────────

async fn deepseek(p: &ProviderConfig) -> Result<QuotaResult, String> {
    let key = require_key(p)?;
    let (status, text) = http_get(
        "https://api.deepseek.com/user/balance",
        &[
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ],
    )
    .await?;
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(auth_failed(status));
    }
    if !status.is_success() {
        return Err(format!("HTTP {status}：{}", truncate(&text, 200)));
    }
    let body = parse_json(&text)?;
    let t = deepseek_balance_text(&body)?;
    Ok(QuotaResult {
        ok: true,
        plan: None,
        tiers: vec![],
        balance_text: Some(t),
        error: None,
        ts: 0,
    })
}

/// 同一账号会列出 CNY / USD 两个币种钱包，扣费走有钱的那个。只要还有币种有余额，
/// 就不再列出空钱包：否则空的那一行会显示「余额不足」，两行还让人误以为有两个钱包
///（移植自 cc-switch #8170）。全部为 0 或全部有余额时原样列出。
fn deepseek_balance_text(body: &serde_json::Value) -> Result<String, String> {
    let is_available = body
        .get("is_available")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let mut wallets = Vec::new();
    if let Some(infos) = body.get("balance_infos").and_then(|v| v.as_array()) {
        for info in infos {
            let currency = info
                .get("currency")
                .and_then(|v| v.as_str())
                .unwrap_or("CNY");
            if let Some(total) = info.get("total_balance").and_then(num) {
                wallets.push((currency, total));
            }
        }
    }
    if wallets.is_empty() {
        return Err("响应中没有余额数据".into());
    }
    if wallets.iter().any(|&(_, total)| total > 0.0) {
        wallets.retain(|&(_, total)| total > 0.0);
    }
    let mut t = format!(
        "DeepSeek 余额：{}",
        wallets
            .iter()
            .map(|(c, total)| format!("{c} {total:.2}"))
            .collect::<Vec<_>>()
            .join("；")
    );
    if !is_available {
        t.push_str("（余额不足）");
    }
    Ok(t)
}

async fn stepfun(p: &ProviderConfig) -> Result<QuotaResult, String> {
    let key = require_key(p)?;
    let (status, text) = http_get(
        "https://api.stepfun.com/v1/accounts",
        &[
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ],
    )
    .await?;
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(auth_failed(status));
    }
    if !status.is_success() {
        return Err(format!("HTTP {status}：{}", truncate(&text, 200)));
    }
    let body = parse_json(&text)?;
    let balance = num_field(&body, "balance").unwrap_or(0.0);
    Ok(QuotaResult {
        ok: true,
        plan: None,
        tiers: vec![],
        balance_text: Some(format!("StepFun 余额：{balance:.2} CNY")),
        error: None,
        ts: 0,
    })
}

async fn siliconflow(p: &ProviderConfig, is_cn: bool) -> Result<QuotaResult, String> {
    let key = require_key(p)?;
    let domain = if is_cn {
        "api.siliconflow.cn"
    } else {
        "api.siliconflow.com"
    };
    let (status, text) = http_get(
        &format!("https://{domain}/v1/user/info"),
        &[
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ],
    )
    .await?;
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(auth_failed(status));
    }
    if !status.is_success() {
        return Err(format!("HTTP {status}：{}", truncate(&text, 200)));
    }
    let body = parse_json(&text)?;
    let data = body
        .get("data")
        .ok_or_else(|| "响应缺少 data 字段".to_string())?;
    let total = num_field(data, "totalBalance").unwrap_or(0.0);
    let unit = if is_cn { "CNY" } else { "USD" };
    let label = if is_cn {
        "SiliconFlow"
    } else {
        "SiliconFlow (EN)"
    };
    Ok(QuotaResult {
        ok: true,
        plan: None,
        tiers: vec![],
        balance_text: Some(format!("{label} 余额：{total:.2} {unit}")),
        error: None,
        ts: 0,
    })
}

async fn openrouter(p: &ProviderConfig) -> Result<QuotaResult, String> {
    let key = require_key(p)?;
    let (status, text) = http_get(
        "https://openrouter.ai/api/v1/credits",
        &[
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ],
    )
    .await?;
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(auth_failed(status));
    }
    if !status.is_success() {
        return Err(format!("HTTP {status}：{}", truncate(&text, 200)));
    }
    let body = parse_json(&text)?;
    parse_openrouter_balance(&body).map(|balance_text| QuotaResult {
        ok: true,
        plan: None,
        tiers: vec![],
        balance_text: Some(balance_text),
        error: None,
        ts: 0,
    })
}

/// /credits 返回的是购买额度而非 Key 有效性：免费账户余额为 0 仍可用免费模型，
/// 不影响展示。字段缺失或非有限数值一律报错，绝不静默按 0 显示（cc-switch #8175）。
fn parse_openrouter_balance(body: &serde_json::Value) -> Result<String, String> {
    let data = body.get("data").unwrap_or(body);
    let (Some(total_credits), Some(total_usage)) = (
        num_field(data, "total_credits").filter(|v| v.is_finite()),
        num_field(data, "total_usage").filter(|v| v.is_finite()),
    ) else {
        return Err("响应缺少或含非法的 total_credits / total_usage".into());
    };
    let remaining = total_credits - total_usage;
    if !remaining.is_finite() {
        return Err("OpenRouter 余额数值非法".into());
    }
    Ok(format!(
        "OpenRouter 余额：{remaining:.2} USD（充值 {total_credits:.2}，已用 {total_usage:.2}）"
    ))
}

async fn novita(p: &ProviderConfig) -> Result<QuotaResult, String> {
    let key = require_key(p)?;
    let (status, text) = http_get(
        "https://api.novita.ai/v3/user/balance",
        &[
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ],
    )
    .await?;
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(auth_failed(status));
    }
    if !status.is_success() {
        return Err(format!("HTTP {status}：{}", truncate(&text, 200)));
    }
    let body = parse_json(&text)?;
    parse_novita_balance(&body).map(|balance_text| QuotaResult {
        ok: true,
        plan: None,
        tiers: vec![],
        balance_text: Some(balance_text),
        error: None,
        ts: 0,
    })
}

/// 金额单位为 0.0001 USD，除以 10000 转为 USD。余额为 0 不代表 Key 失效；
/// 字段缺失或非法（含 NaN/inf）直接报错，不静默按 0 显示（移植自 cc-switch）。
fn parse_novita_balance(body: &serde_json::Value) -> Result<String, String> {
    let Some(available) = num_field(body, "availableBalance")
        .map(|v| v / 10000.0)
        .filter(|v| v.is_finite())
    else {
        return Err("响应缺少或含非法的 availableBalance".into());
    };
    Ok(format!("Novita AI 余额：{available:.2} USD"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_kind() {
        use Kind::*;
        assert_eq!(
            detect_kind("https://open.bigmodel.cn/api/anthropic"),
            Some(Zhipu)
        );
        assert_eq!(detect_kind("https://api.z.ai/api/anthropic"), Some(Zhipu));
        assert_eq!(detect_kind("https://api.kimi.com/coding/"), Some(Kimi));
        assert_eq!(
            detect_kind("https://api.minimax.cn/anthropic"),
            Some(MiniMaxCn)
        );
        assert_eq!(
            detect_kind("https://api.minimax.io/anthropic"),
            Some(MiniMaxEn)
        );
        assert_eq!(detect_kind("https://opencode.ai/zen/go"), Some(OpencodeGo));
        assert_eq!(
            detect_kind("https://ark.cn-beijing.volces.com/api/plan/v3"),
            Some(Volcengine)
        );
        assert_eq!(
            detect_kind("https://api.deepseek.com/anthropic"),
            Some(DeepSeek)
        );
        assert_eq!(
            detect_kind("https://api.siliconflow.cn/v1"),
            Some(SiliconFlowCn)
        );
        assert_eq!(
            detect_kind("https://openrouter.ai/api/v1"),
            Some(OpenRouter)
        );
        assert_eq!(detect_kind("https://api.novita.ai/v3"), Some(Novita));
        assert_eq!(detect_kind("https://api.moonshot.cn/anthropic"), None);
    }

    #[test]
    fn test_zhipu_tiers() {
        let body: serde_json::Value = serde_json::from_str(
            r#"{"success":true,"data":{"level":"GLM Coding Pro","limits":[
                {"type":"TOKENS_LIMIT","unit":3,"percentage":12.5,"nextResetTime":1759000000000},
                {"type":"TOKENS_LIMIT","unit":6,"percentage":45.0,"nextResetTime":1759500000000}
            ]}}"#,
        )
        .unwrap();
        let (tiers, plan) = zhipu_tiers_from_body(&body).unwrap();
        assert_eq!(plan.as_deref(), Some("GLM Coding Pro"));
        assert_eq!(tiers[0].name, "五小时窗口");
        assert!((tiers[0].utilization - 12.5).abs() < 1e-9);
        assert_eq!(tiers[1].name, "本周额度");
        assert!(tiers[0].resets_at.is_some());
        assert_eq!(tiers[0].resets_at_ts, Some(1759000000));
    }

    #[test]
    fn test_reset_display() {
        // 秒级时间戳
        let v = serde_json::json!(1759000000i64);
        assert!(reset_display(&v).is_some());
        // ISO 字符串
        let v = serde_json::json!("2026-09-28T15:00:00Z");
        assert!(reset_display(&v).is_some());
        // 0 / 负数视为无
        assert!(reset_display(&serde_json::json!(0)).is_none());
        assert!(reset_display(&serde_json::json!(-1)).is_none());
        // 非法
        assert!(reset_display(&serde_json::json!("abc")).is_none());
    }

    #[test]
    fn test_reset_ts() {
        // 秒级 / 毫秒级时间戳归一到秒
        assert_eq!(
            reset_ts(&serde_json::json!(1759000000i64)),
            Some(1759000000)
        );
        assert_eq!(
            reset_ts(&serde_json::json!(1759000000000i64)),
            Some(1759000000)
        );
        // ISO 字符串
        assert_eq!(
            reset_ts(&serde_json::json!("2026-09-28T15:00:00Z")),
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-09-28T15:00:00Z")
                    .unwrap()
                    .timestamp()
            )
        );
        // 0 / 负数 / 非法 / 异常小的值视为无
        assert_eq!(reset_ts(&serde_json::json!(0)), None);
        assert_eq!(reset_ts(&serde_json::json!(-1)), None);
        assert_eq!(reset_ts(&serde_json::json!("abc")), None);
        assert_eq!(reset_ts(&serde_json::json!(123456i64)), None);
    }

    #[test]
    fn test_volc_uri_encode() {
        assert_eq!(volc_uri_encode("a b+c/d"), "a%20b%2Bc%2Fd");
        assert_eq!(volc_uri_encode("Action"), "Action");
        assert_eq!(volc_uri_encode("a~b-c_d.e"), "a~b-c_d.e");
    }

    #[test]
    fn test_volc_region() {
        assert_eq!(
            volc_region("https://ark.cn-beijing.volces.com/api/plan/v3"),
            "cn-beijing"
        );
        assert_eq!(
            volc_region("https://ark.ap-southeast.volces.com/api/coding"),
            "ap-southeast"
        );
        assert_eq!(volc_region("https://example.com"), VOLC_DEFAULT_REGION);
    }

    #[test]
    fn test_volc_coding_window() {
        assert_eq!(volc_window("session"), Some("五小时窗口"));
        assert_eq!(volc_window("weekly"), Some("本周额度"));
        assert_eq!(volc_window("monthly"), Some("本月额度"));
        assert_eq!(volc_window("daily"), None);
    }

    #[test]
    fn test_volc_probe_order() {
        // Coding 端点先查 Coding Plan；其余（含 /api/plan、未知端点）保持旧行为先查 Agent
        let (coding, agent) = (VolcPlan::Coding, VolcPlan::Afp);
        assert_eq!(
            volc_probe_order("https://ark.cn-beijing.volces.com/api/coding"),
            (coding, agent)
        );
        assert_eq!(
            volc_probe_order("https://ark.cn-beijing.volces.com/api/plan"),
            (agent, coding)
        );
        assert_eq!(
            volc_probe_order("https://example.com/whatever"),
            (agent, coding)
        );
    }

    /// 2026-11 真实接口返回结构（GetCodingPlanUsage），锁住解析器字段
    #[test]
    fn test_volc_coding_plan_parse_real_shape() {
        let result = serde_json::json!({
            "Status": "Running",
            "UpdateTimestamp": 1793688128,
            "QuotaUsage": [
                { "Level": "session", "Percent": 2.513238, "ResetTimestamp": 1793705487,
                  "Cap": 100, "RewardTotalPercent": 0 },
                { "Level": "weekly", "Percent": 0.3350984, "ResetTimestamp": 1794153600,
                  "Cap": 100, "RewardTotalPercent": 0 },
                { "Level": "monthly", "Percent": 0.1675492, "ResetTimestamp": 1796313599,
                  "Cap": 100, "RewardTotalPercent": 0 }
            ],
            "HasReward": false
        });
        let tiers = parse_coding_plan_tiers(&result);
        assert_eq!(tiers.len(), 3);
        assert_eq!(tiers[0].name, "五小时窗口");
        assert!((tiers[0].utilization - 2.513238).abs() < 1e-9);
        assert_eq!(tiers[1].name, "本周额度");
        assert_eq!(tiers[2].name, "本月额度");
        // 秒级 ResetTimestamp 归一到秒
        assert_eq!(tiers[0].resets_at_ts, Some(1793705487));
    }

    #[test]
    fn test_detect_command_code() {
        use Kind::*;
        assert_eq!(
            detect_kind("https://api.commandcode.ai/provider"),
            Some(CommandCode)
        );
        assert_eq!(
            detect_kind("https://api.commandcode.ai/provider/v1"),
            Some(CommandCode)
        );
    }

    fn cc_credits_body(limited: bool, nested: bool) -> serde_json::Value {
        let window_limits = serde_json::json!({
            "limited": limited,
            "fiveHour": { "used": 2.5, "cap": 10.0, "resetAt": "2026-10-13T03:00:00Z" },
            "weekly": { "used": 30.0, "cap": 100.0, "resetAt": "2026-10-19T00:00:00Z" },
        });
        let mut root = serde_json::json!({
            "credits": {
                "monthlyCredits": 60.0,
                "purchasedCredits": 30.0,
                "freeCredits": 10.0,
                "planId": "pro",
            }
        });
        if nested {
            // 历史变体：windowLimits 嵌在 credits 里
            root["credits"]["windowLimits"] = window_limits;
        } else {
            // 实际响应把 windowLimits 放在 credits 同级
            root["windowLimits"] = window_limits;
        }
        root
    }

    fn cc_summary(spent: f64) -> serde_json::Value {
        serde_json::json!({ "totalCost": spent })
    }

    #[test]
    fn test_command_code_limited_shows_all_windows() {
        let subscription = serde_json::json!({
            "data": { "planId": "pro-max", "currentPeriodEnd": "2026-11-01T00:00:00Z" }
        });
        let r = parse_command_code_quota(
            &cc_credits_body(true, false),
            Some(&subscription),
            &cc_summary(25.0),
        )
        .unwrap();
        assert_eq!(r.plan.as_deref(), Some("pro-max"));
        assert_eq!(r.tiers.len(), 3);
        assert_eq!(r.tiers[0].name, "五小时窗口");
        assert!((r.tiers[0].utilization - 25.0).abs() < 1e-9);
        assert!(r.tiers[0].resets_at.is_some() && r.tiers[0].resets_at_ts.is_some());
        assert_eq!(r.tiers[1].name, "本周额度");
        assert!((r.tiers[1].utilization - 30.0).abs() < 1e-9);
        // 月度池 = 已花 25 /（已花 25 + 剩余 100）= 20%
        assert_eq!(r.tiers[2].name, "本月额度");
        assert!((r.tiers[2].utilization - 20.0).abs() < 1e-9);
        assert!(r.tiers[2].resets_at.is_some() && r.tiers[2].resets_at_ts.is_some());
    }

    #[test]
    fn test_command_code_unlimited_only_monthly_and_nested_variant() {
        let r = parse_command_code_quota(&cc_credits_body(false, true), None, &cc_summary(0.0))
            .unwrap();
        assert_eq!(r.tiers.len(), 1);
        assert_eq!(r.tiers[0].name, "本月额度");
        assert_eq!(r.tiers[0].utilization, 0.0);
        // 无订阅响应时 planId 回退到 credits 内
        assert_eq!(r.plan.as_deref(), Some("pro"));
        assert!(r.tiers[0].resets_at.is_none());
    }

    #[test]
    fn test_command_code_rejects_missing_fields() {
        for (credits, summary) in [
            (serde_json::json!({}), cc_summary(1.0)),
            (cc_credits_body(true, false), serde_json::json!({})),
            (serde_json::json!({ "credits": null }), cc_summary(1.0)),
        ] {
            assert!(
                parse_command_code_quota(&credits, None, &summary).is_err(),
                "should reject {credits}"
            );
        }
    }

    #[test]
    fn test_openrouter_balance_parse() {
        // 正常 / 数值字符串 / 无 data 包裹 三种形态都要出正确金额
        for body in [
            serde_json::json!({ "data": { "total_credits": 100.0, "total_usage": 25.5 } }),
            serde_json::json!({ "total_credits": "100.00", "total_usage": "25.50" }),
        ] {
            let text = parse_openrouter_balance(&body).unwrap();
            assert!(text.contains("余额：74.50 USD"), "{text}");
            assert!(text.contains("充值 100.00"), "{text}");
        }
        // 免费账户：余额为 0 是合法展示，不是错误
        let text = parse_openrouter_balance(&serde_json::json!(
            { "data": { "total_credits": 0.0, "total_usage": 0.0 } }
        ))
        .unwrap();
        assert!(text.contains("余额：0.00 USD"), "{text}");
        // 字段缺失 / 非法 / 非有限值 一律报错
        for body in [
            serde_json::json!({}),
            serde_json::json!({ "data": null }),
            serde_json::json!({ "data": { "limit": 100, "limit_remaining": 100 } }),
            serde_json::json!({ "data": { "total_credits": 100.0 } }),
            serde_json::json!({ "data": { "total_credits": "NaN", "total_usage": 0.0 } }),
            serde_json::json!({ "data": { "total_credits": 1e308, "total_usage": -1e308 } }),
        ] {
            assert!(parse_openrouter_balance(&body).is_err(), "{body}");
        }
    }

    #[test]
    fn test_novita_balance_parse() {
        for (raw, expect) in [
            (0.0, "余额：0.00 USD"),
            (-50000.0, "余额：-5.00 USD"),
            (255000.0, "余额：25.50 USD"),
        ] {
            let text =
                parse_novita_balance(&serde_json::json!({ "availableBalance": raw })).unwrap();
            assert!(text.contains(expect), "{text}");
        }
        let text =
            parse_novita_balance(&serde_json::json!({ "availableBalance": "10000" })).unwrap();
        assert!(text.contains("余额：1.00 USD"), "{text}");
        for body in [
            serde_json::json!({}),
            serde_json::json!({ "availableBalance": null }),
            serde_json::json!({ "availableBalance": "invalid" }),
            serde_json::json!({ "availableBalance": "inf" }),
            serde_json::json!({ "cashBalance": 10000 }),
        ] {
            assert!(parse_novita_balance(&body).is_err(), "{body}");
        }
    }

    // ── 智谱：无套餐识别 / 余额解析（响应体为 cc-switch 2026-10-18 实测原文）──

    const ZHIPU_NO_PLAN_BODY: &str =
        r#"{"code":500,"msg":"当前用户不存在coding plan","success":false}"#;

    const ZHIPU_BALANCE_BODY: &str = r#"{"code":200,"msg":"操作成功","data":{"balance":14.68002004,"rechargeAmount":15.0,"giveAmount":0.0,"totalSpendAmount":0.31997996,"todaySpendAmount":null,"availableBalance":14.68002004,"frozenBalance":0.0,"creditBalance":null,"availableCreditBalance":null,"creditStatus":"NOT_OPEN","modelSpendAmountList":null,"isKA":false},"success":true}"#;

    #[test]
    fn test_zhipu_no_plan_detection() {
        let no_plan: serde_json::Value = serde_json::from_str(ZHIPU_NO_PLAN_BODY).unwrap();
        assert!(zhipu_body_has_no_plan(&no_plan));

        // 英文文案未实测，关键词放宽后要能认出
        for body in [
            serde_json::json!({ "code": 500, "msg": "The current user does not have a coding plan", "success": false }),
            serde_json::json!({ "code": 500, "msg": "Coding Plan not exist", "success": false }),
        ] {
            assert!(zhipu_body_has_no_plan(&body), "{body}");
        }

        // 其他业务错误、提到套餐但不是「没有」、success 不是 false 都不算没套餐
        for body in [
            serde_json::json!({ "code": 1001, "msg": "令牌已过期或验证不正确", "success": false }),
            serde_json::json!({ "code": 500, "msg": "coding plan 查询超时", "success": false }),
            serde_json::json!({ "code": 500, "msg": "当前用户不存在coding plan" }),
            serde_json::json!({ "success": true, "data": { "limits": [] } }),
        ] {
            assert!(!zhipu_body_has_no_plan(&body), "{body}");
        }
    }

    #[test]
    fn test_zhipu_balance_parse() {
        let body: serde_json::Value = serde_json::from_str(ZHIPU_BALANCE_BODY).unwrap();
        let text = parse_zhipu_balance(&body, true).unwrap();
        assert!(text.contains("智谱 GLM 余额：14.68 CNY"), "{text}");

        // 国际站未实测：同结构、币种记 USD；availableBalance 缺失时退回 balance，数字串也认
        let text = parse_zhipu_balance(
            &serde_json::json!({ "success": true, "data": { "balance": "3.5", "frozenBalance": 0 } }),
            false,
        )
        .unwrap();
        assert!(text.contains("Z.ai 余额：3.50 USD"), "{text}");

        // 用完不算错误，照常显示 0
        let text = parse_zhipu_balance(
            &serde_json::json!({ "data": { "availableBalance": 0 } }),
            true,
        )
        .unwrap();
        assert!(text.contains("余额：0.00 CNY"), "{text}");

        // 业务错误、缺金额、非有限值一律报错
        for body in [
            serde_json::json!({ "code": 401, "msg": "令牌已过期或验证不正确", "success": false }),
            serde_json::json!({ "code": 200, "success": true, "data": null }),
            serde_json::json!({ "code": 200, "success": true, "data": { "availableBalance": "inf" } }),
            serde_json::json!({}),
        ] {
            assert!(parse_zhipu_balance(&body, true).is_err(), "{body}");
        }
    }

    #[test]
    fn test_deepseek_balance_text() {
        // 一个币种还有钱时不再列出空钱包（cc-switch #8170）
        let text = deepseek_balance_text(&serde_json::json!({
            "is_available": true,
            "balance_infos": [
                { "currency": "CNY", "total_balance": "0.00" },
                { "currency": "USD", "total_balance": "12.34" }
            ]
        }))
        .unwrap();
        assert!(text.contains("USD 12.34"), "{text}");
        assert!(!text.contains("CNY"), "{text}");

        // 全为 0 / 全有钱：原样列出
        let text = deepseek_balance_text(&serde_json::json!({ "balance_infos": [
            { "currency": "CNY", "total_balance": "0.00" },
            { "currency": "USD", "total_balance": "0" }
        ] }))
        .unwrap();
        assert!(
            text.contains("CNY 0.00") && text.contains("USD 0.00"),
            "{text}"
        );

        let text = deepseek_balance_text(&serde_json::json!({ "balance_infos": [
            { "currency": "CNY", "total_balance": "5.00" },
            { "currency": "USD", "total_balance": "1.00" }
        ] }))
        .unwrap();
        assert!(
            text.contains("CNY 5.00") && text.contains("USD 1.00"),
            "{text}"
        );

        // is_available=false 附「余额不足」
        let text = deepseek_balance_text(&serde_json::json!({
            "is_available": false,
            "balance_infos": [{ "currency": "CNY", "total_balance": "0.50" }]
        }))
        .unwrap();
        assert!(text.ends_with("（余额不足）"), "{text}");

        // 无余额数据报错
        assert!(deepseek_balance_text(&serde_json::json!({})).is_err());
        assert!(deepseek_balance_text(&serde_json::json!({ "balance_infos": [] })).is_err());
        assert!(deepseek_balance_text(
            &serde_json::json!({ "balance_infos": [{ "currency": "CNY" }] })
        )
        .is_err());
    }
}
