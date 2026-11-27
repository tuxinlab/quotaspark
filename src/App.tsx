import { useCallback, useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, type LogEntry, type ProviderCard, type ProviderConfig, type SyncReport, type QuotaTier } from "./api";
import ProviderForm, { QUOTA_SUPPORTED_HINT } from "./ProviderForm";
import UpdateButton from "./UpdateButton";
import "./App.css";

function fmtTime(ts: number) {
  return new Date(ts).toLocaleTimeString("zh-CN", { hour12: false });
}

/** 档位带重置时间时每 30s 重渲染一次，让「剩余多久重置」保持新鲜 */
function useMinuteTicker(active: boolean) {
  const [, setTick] = useState(0);
  useEffect(() => {
    if (!active) return;
    const id = window.setInterval(() => setTick((n) => n + 1), 30_000);
    return () => window.clearInterval(id);
  }, [active]);
}

// 剩余时长展示：3天2小时 / 3小时12分 / 12分钟 / 不到1分钟；已过返回 null
function fmtRemaining(sec: number): string | null {
  if (!(sec > 0)) return null;
  const d = Math.floor(sec / 86400);
  const h = Math.floor((sec % 86400) / 3600);
  const m = Math.floor((sec % 3600) / 60);
  if (d > 0) return `${d}天${h}小时`;
  if (h > 0) return `${h}小时${m}分`;
  if (m > 0) return `${m}分钟`;
  return "不到1分钟";
}

function TierBar({ tier }: { tier: QuotaTier }) {
  useMinuteTicker(tier.resets_at_ts != null);
  const pct = Math.min(100, Math.max(0, tier.utilization));
  const hot = pct >= 90;
  const remaining =
    tier.resets_at_ts != null ? fmtRemaining(tier.resets_at_ts - Date.now() / 1000) : null;
  const sub = [
    tier.resets_at ? `${tier.resets_at} 重置` : "",
    remaining ? `剩余${remaining}` : "",
    tier.amount ?? "",
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <div className="tier">
      <div className="tier-head">
        <span>{tier.name}</span>
        <span className={hot ? "pct hot" : "pct"}>{tier.utilization.toFixed(0)}%</span>
      </div>
      <div className="bar">
        <div className={hot ? "bar-fill hot" : "bar-fill"} style={{ width: `${pct}%` }} />
      </div>
      {sub && <div className="tier-reset">{sub}</div>}
    </div>
  );
}

/* ── 主题切换：跟随系统 → 深 → 浅 循环，localStorage 持久化 ── */

type ThemeMode = "auto" | "dark" | "light";
const THEME_ORDER: ThemeMode[] = ["auto", "dark", "light"];

function useThemeCycle(): [ThemeMode, () => void] {
  const [mode, setMode] = useState<ThemeMode>(() => {
    const saved = localStorage.getItem("qs-theme");
    return saved === "dark" || saved === "light" || saved === "auto" ? saved : "auto";
  });
  useEffect(() => {
    localStorage.setItem("qs-theme", mode);
    const mq = window.matchMedia("(prefers-color-scheme: light)");
    const apply = () => {
      document.documentElement.dataset.theme =
        mode === "auto" ? (mq.matches ? "light" : "dark") : mode;
    };
    apply();
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  }, [mode]);
  return [mode, () => setMode(THEME_ORDER[(THEME_ORDER.indexOf(mode) + 1) % 3])];
}

const SunIcon = () => (
  <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <circle cx="12" cy="12" r="4" />
    <path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
  </svg>
);

const MoonIcon = () => (
  <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M21 12.8A9 9 0 1 1 11.2 3 7 7 0 0 0 21 12.8z" />
  </svg>
);

const AutoIcon = () => (
  <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
    <circle cx="12" cy="12" r="9" />
    <path d="M12 3a9 9 0 0 1 0 18z" fill="currentColor" stroke="none" />
  </svg>
);

const GitHubIcon = () => (
  <svg width="15" height="15" viewBox="0 0 16 16" fill="currentColor" aria-hidden>
    <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8z" />
  </svg>
);

function FlameLogo() {
  return (
    <svg className="mark" viewBox="0 0 24 24" fill="none" aria-hidden>
      <path
        d="M12.2 2c.4 3.4-.9 5.2-2.6 7C7.9 10.8 6 12.8 6 15.7a6 6 0 0 0 12 0c0-2.3-1-4.1-2.3-5.7-.6 1.1-1.4 1.9-2.4 2.3.4-3.5-.2-7-1.1-10.3z"
        fill="currentColor"
      />
      <path
        d="M12 21.8a3.4 3.4 0 0 1-3.4-3.4c0-1.7 1.2-2.9 2.3-4 .5.9 1.3 1.5 2.2 1.7-.2 1.9.4 3 1.9 3.9-.5 1.1-1.6 1.8-3 1.8z"
        fill="#fff"
        opacity="0.22"
      />
    </svg>
  );
}

/* ── 调度模板 ─────────────────────────────────────────────── */

const GITHUB_URL = "https://github.com/tuxinlab/quotaspark";

const TEMPLATES: { name: string; desc: string; times: string[] }[] = [
  {
    name: "全天接力",
    desc: "05:30 / 10:30 / 15:30 / 20:30（四棒覆盖到次日 01:30）",
    times: ["05:30", "10:30", "15:30", "20:30"],
  },
];

/* ── 卡片上的调度行：开关 + 时间 chips 就地编辑 + 模板菜单 ── */

function SchedRow({ p, onChanged }: { p: ProviderCard; onChanged: () => void }) {
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState("");
  const [editIdx, setEditIdx] = useState<number | null>(null);
  const [tplOpen, setTplOpen] = useState(false);
  const [infoOpen, setInfoOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const tplRef = useRef<HTMLDivElement>(null);
  const infoRef = useRef<HTMLDivElement>(null);

  // 点外关闭。不用 fixed 遮罩：卡片 hover 有 transform，会把 fixed 后代的
  // 包含块压缩到卡片内，导致"点外面关闭"失效、浮层卡死
  useEffect(() => {
    if (!tplOpen && !infoOpen) return;
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (tplOpen && tplRef.current && !tplRef.current.contains(t)) setTplOpen(false);
      if (infoOpen && infoRef.current && !infoRef.current.contains(t)) setInfoOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [tplOpen, infoOpen]);

  const save = (times: string[], enabled?: boolean) =>
    api.setSchedule(p.id, enabled ?? p.enabled, times).then(onChanged).catch(() => {});

  const closeInput = () => {
    setAdding(false);
    setEditIdx(null);
    setDraft("");
  };

  const commit = () => {
    const v = draft.trim();
    closeInput();
    if (!/^\d{1,2}:\d{2}$/.test(v)) return;
    const norm = v.length === 4 ? `0${v}` : v;
    const times =
      editIdx === null
        ? [...p.times, norm].sort()
        : p.times.map((t, i) => (i === editIdx ? norm : t)).sort();
    if (JSON.stringify(times) !== JSON.stringify(p.times)) save(times);
  };

  useEffect(() => {
    if ((adding || editIdx !== null) && inputRef.current) inputRef.current.focus();
  }, [adding, editIdx]);

  const inputEl = (
    <input
      ref={inputRef}
      type="time"
      className="chip-input"
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
        if (e.key === "Escape") closeInput();
      }}
      data-testid="time-input"
    />
  );

  return (
    <div className="sched" data-testid="sched-row">
      <label className="switch" title="是否启用定时激活">
        <input
          type="checkbox"
          checked={p.enabled}
          data-testid="sched-toggle"
          onChange={(e) => save(p.times, e.target.checked)}
        />
        定时
      </label>

      <div className="chips" data-testid="time-chips">
        {p.times.map((t, i) =>
          editIdx === i ? (
            inputEl
          ) : (
            <span className="chip" key={`${t}-${i}`}>
              <button
                className="chip-time"
                title="点击修改"
                onClick={() => {
                  setEditIdx(i);
                  setDraft(t);
                }}
              >
                {t}
              </button>
              <button
                className="chip-x"
                title="删除该时间"
                onClick={() => save(p.times.filter((_, j) => j !== i))}
              >
                ×
              </button>
            </span>
          ),
        )}
        {adding ? (
          inputEl
        ) : (
          <button
            className="chip-add"
            title="添加触发时间"
            data-testid="chip-add"
            onClick={() => {
              setEditIdx(null);
              setDraft("");
              setAdding(true);
            }}
          >
            ＋
          </button>
        )}
        {!p.times.length && !adding && <span className="chips-empty">未设置触发时间</span>}
      </div>

      <div className="tpl-wrap" ref={tplRef}>
        <button
          className="tpl-btn"
          title="套用调度模板"
          data-testid="tpl-btn"
          onClick={() => setTplOpen(v => !v)}
        >
          模板
        </button>
        {tplOpen && (
          <div className="tpl-menu" data-testid="tpl-menu">
            {TEMPLATES.map((t) => (
              <button
                key={t.name}
                className="tpl-item"
                onClick={() => {
                  setTplOpen(false);
                  save(t.times, true);
                }}
              >
                <span className="tpl-name">{t.name}</span>
                <span className="tpl-desc">{t.desc}</span>
              </button>
            ))}
          </div>
        )}
      </div>

      <div className="info-wrap" ref={infoRef}>
        <button
          className="info-btn"
          title="调度说明"
          data-testid="sched-info"
          onClick={() => setInfoOpen(v => !v)}
        >
          ⓘ
        </button>
        {infoOpen && (
          <div className="tpl-menu info-pop" data-testid="sched-info-pop">
            <p className="info-title">调度说明</p>
            <ul className="info-list">
              <li>到点后自动发送一条最小请求（max_tokens=1），成功即点燃该供应商的五小时窗口</li>
              <li>五小时从点燃时刻滚动计算；时间点间隔设为 5 小时（如 05:30/10:30/15:30/20:30），窗口即可全天首尾相接</li>
              <li>应用重启后 10 分钟内错过的时点会补发一次，去重保证不会重复发送</li>
              <li>调度仅在应用运行时生效；默认已开启开机自启，重启后自动后台运行并补触发（顶栏可关闭）</li>
            </ul>
          </div>
        )}
      </div>
    </div>
  );
}

export default function App() {
  const [providers, setProviders] = useState<ProviderCard[]>([]);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [editing, setEditing] = useState<ProviderConfig | null | undefined>(undefined);
  const [notice, setNotice] = useState("");
  const [confirmDel, setConfirmDel] = useState<ProviderCard | null>(null);
  const [autostart, setAutostart] = useState(false);
  const [querying, setQuerying] = useState<Record<string, boolean>>({});
  const [refreshingAll, setRefreshingAll] = useState(false);
  // 卡片拖拽排序：拖拽源是手柄自身（常驻 draggable；整卡 draggable 会废掉卡内
  // 文本选择，而"按下后才翻转属性"在 WebView2 原生拖拽里来不及——按下瞬间就做
  // 命中判定）。dragId=拖拽中的卡，overId=当前悬停的落点卡。
  const [dragId, setDragId] = useState<string | null>(null);
  const [overId, setOverId] = useState<string | null>(null);
  const [themeMode, cycleTheme] = useThemeCycle();
  const themeLabel = themeMode === "auto" ? "跟随系统" : themeMode === "dark" ? "深色" : "浅色";

  const refresh = useCallback(async () => {
    try {
      setProviders(await api.getProviders());
      setLogs(await api.getLogs());
    } catch (e) {
      console.error(e);
    }
  }, []);

  useEffect(() => {
    refresh();
    api.getAutostart().then(setAutostart).catch(() => {});
    const un = api.onChanged(() => {
      refresh();
    });
    return () => {
      un.then((f) => f());
    };
  }, [refresh]);

  const showNotice = (msg: string) => {
    setNotice(msg);
    window.setTimeout(() => setNotice(""), 5000);
  };

  // 手动查询额度：后端查完会 emit state-changed 触发 refresh，这里只负责
  // 按住按钮的"查询中…"反馈，防止连点重复发起请求。
  const runQuotaQuery = async (id: string) => {
    if (querying[id]) return;
    setQuerying((m) => ({ ...m, [id]: true }));
    try {
      await api.queryQuota(id);
    } catch (e) {
      console.error(e);
    } finally {
      setQuerying((m) => ({ ...m, [id]: false }));
    }
  };

  // 全部刷新：并行重查所有支持额度的供应商，各卡片同时进入"查询中…"。
  // 单个失败不影响其余（结果由后端写入状态后逐个 emit 刷新）。
  const onRefreshAll = async () => {
    if (refreshingAll) return;
    const ids = providers.filter((p) => p.supports_quota).map((p) => p.id);
    if (!ids.length) {
      showNotice("没有支持自动查额度的供应商");
      return;
    }
    setRefreshingAll(true);
    setQuerying((m) => Object.fromEntries([...Object.entries(m), ...ids.map((id) => [id, true])]));
    try {
      await Promise.allSettled(ids.map((id) => api.queryQuota(id)));
      showNotice(`已刷新 ${ids.length} 个供应商的额度`);
    } finally {
      setQuerying((m) => Object.fromEntries(Object.entries(m).filter(([k]) => !ids.includes(k))));
      setRefreshingAll(false);
      refresh();
    }
  };

  // 拖拽落点：把被拖卡插到落点卡的位置（落点卡顺延）。先乐观更新再持久化，
  // 后端存完会 emit state-changed 走 refresh 兜底。
  const commitOrder = async (sourceId: string, targetId: string) => {
    if (sourceId === targetId) return;
    const src = providers.find((p) => p.id === sourceId);
    if (!src) return;
    const next = providers.filter((p) => p.id !== sourceId);
    const at = next.findIndex((p) => p.id === targetId);
    if (at < 0) return;
    next.splice(at, 0, src);
    setProviders(next);
    try {
      await api.reorderProviders(next.map((p) => p.id));
    } catch (e) {
      console.error(e);
      refresh();
    }
  };

  const onSync = async () => {
    try {
      const r: SyncReport = await api.syncFromCc();
      showNotice(
        `同步完成：导入 ${r.imported} 个、更新 ${r.updated} 个、跳过 ${r.skipped} 个（官方登录或本地路由占位不含 Key）`,
      );
    } catch (e) {
      showNotice(`同步失败：${e}`);
    }
  };

  const onActivateAll = async () => {
    if (!providers.length) {
      showNotice("还没有供应商，请先同步或手动添加");
      return;
    }
    await api.activateNow();
    showNotice("已开始激活全部供应商，结果见日志");
  };

  const toggleAutostart = async () => {
    const next = !autostart;
    try {
      await api.setAutostart(next);
      setAutostart(next);
      showNotice(next ? "已开启开机自启：登录后自动后台运行" : "已关闭开机自启");
    } catch (e) {
      showNotice(`设置开机自启失败：${e}`);
    }
  };

  return (
    <div className="app">
      <div className="topbar">
        <div className="brand">
          <FlameLogo />
          <div className="brand-text">
            <h1>
              额度火花 <span className="en">QuotaSpark</span>
            </h1>
            <p className="subtitle">定时点燃 Coding Plan 的五小时额度窗口</p>
          </div>
        </div>
        <div className="topbar-actions">
          <button
            className="icon-btn"
            title="GitHub 仓库"
            data-testid="github-btn"
            onClick={() => openUrl(GITHUB_URL).catch(() => window.open(GITHUB_URL, "_blank"))}
          >
            <GitHubIcon />
          </button>
          <button
            className="icon-btn"
            title={`主题：${themeLabel}（点击切换）`}
            data-testid="theme-toggle"
            onClick={cycleTheme}
          >
            {themeMode === "auto" ? <AutoIcon /> : themeMode === "dark" ? <MoonIcon /> : <SunIcon />}
          </button>
          <button
            className={autostart ? "toggle-on" : ""}
            title={autostart ? "开机自启：已开启（点击关闭）" : "开机自启：已关闭（点击开启）"}
            data-testid="autostart-btn"
            onClick={toggleAutostart}
          >
            {autostart ? "开机自启 ✓" : "开机自启"}
          </button>
          <UpdateButton onNotice={showNotice} />
          <button onClick={onSync} data-testid="sync-btn">同步 cc-switch 配置</button>
          <button onClick={() => setEditing(null)} data-testid="add-btn">＋ 手动添加</button>
          <button
            data-testid="refresh-all-btn"
            title="重新查询全部供应商的额度"
            disabled={refreshingAll}
            onClick={onRefreshAll}
          >
            {refreshingAll ? "刷新中…" : "↻ 全部刷新"}
          </button>
          <button className="primary" onClick={onActivateAll} data-testid="activate-all-btn">全部激活</button>
        </div>
      </div>

      {notice && <div className="notice" data-testid="notice">{notice}</div>}

      <div className="cards">
        {providers.map((p) => (
          <div
            className={
              "card" +
              (dragId === p.id ? " dragging" : "") +
              (dragId && dragId !== p.id && overId === p.id ? " drop-target" : "")
            }
            key={p.id}
            data-testid="provider-card"
            data-name={p.name}
            onDragOver={(e) => {
              if (!dragId || dragId === p.id) return;
              e.preventDefault();
              e.dataTransfer.dropEffect = "move";
              if (overId !== p.id) setOverId(p.id);
            }}
            onDrop={(e) => {
              e.preventDefault();
              // 优先用 state；兜底读 dataTransfer（dragstart 已 setData），
              // 避免 dragstart→drop 间状态未刷出时丢落点
              const src = dragId || e.dataTransfer.getData("text/plain");
              if (src && src !== p.id) commitOrder(src, p.id);
            }}
          >
            <div className="card-head">
              <div className="card-head-left">
                <span
                  className="drag-handle"
                  data-testid="drag-handle"
                  title="拖动调整卡片顺序"
                  draggable
                  onDragStart={(e) => {
                    setDragId(p.id);
                    e.dataTransfer.effectAllowed = "move";
                    e.dataTransfer.setData("text/plain", p.id);
                    // 拖拽影象显示整张卡片（默认只有小小的手柄图形）
                    const card = (e.currentTarget as Element).closest(".card");
                    if (card instanceof HTMLElement) e.dataTransfer.setDragImage(card, 24, 16);
                  }}
                  onDragEnd={() => {
                    setDragId(null);
                    setOverId(null);
                  }}
                >
                  ⠿
                </span>
                <div className="card-title-wrap">
                  <div className="card-title">
                    {p.name}
                    <span className="badge format">{p.format === "openai" ? "OpenAI" : "Anthropic"}</span>
                    {p.cc_provider_id && <span className="badge cc">cc</span>}
                  </div>
                  <div className="card-sub">
                    {p.model} · {p.base_url}
                  </div>
                </div>
              </div>
              {p.enabled && (
                <span className="badge on">
                  <i className="dot" />
                  定时中
                </span>
              )}
            </div>

            <div className="quota" data-testid="quota-panel">
              {!p.supports_quota ? (
                // 域名不在支持清单里：查询必报"未识别"，直接不给查询入口
                <div className="quota-unsupported" title={QUOTA_SUPPORTED_HINT}>
                  该供应商不支持自动查额度（激活不受影响）
                </div>
              ) : p.quota?.ok ? (
                <>
                  {p.quota.tiers.map((t, i) => (
                    <TierBar key={i} tier={t} />
                  ))}
                  {p.quota.balance_text && <div className="balance">{p.quota.balance_text}</div>}
                  <div className="quota-ts">
                    <span>
                      查询于 {fmtTime(p.quota.ts)}{p.quota.plan ? ` · ${p.quota.plan}` : ""}
                    </span>
                    <button
                      className="quota-refresh"
                      data-testid="quota-refresh"
                      title="重新查询额度"
                      disabled={!!querying[p.id]}
                      onClick={() => runQuotaQuery(p.id)}
                    >
                      {querying[p.id] ? "查询中…" : "↻ 刷新"}
                    </button>
                  </div>
                </>
              ) : p.quota ? (
                <div className="quota-err">
                  {p.quota.error}
                  <button
                    className="quota-refresh"
                    data-testid="quota-refresh"
                    disabled={!!querying[p.id]}
                    onClick={() => runQuotaQuery(p.id)}
                  >
                    {querying[p.id] ? "查询中…" : "重试"}
                  </button>
                </div>
              ) : (
                <div className="quota-empty">
                  <span>未查询额度</span>
                  <button
                    data-testid="quota-btn"
                    disabled={!!querying[p.id]}
                    onClick={() => runQuotaQuery(p.id)}
                  >
                    {querying[p.id] ? "查询中…" : "查额度"}
                  </button>
                </div>
              )}
            </div>

            <SchedRow p={p} onChanged={refresh} />

            <div className="card-foot">
              {p.last && (
                <div className={"last " + (p.last.ok ? "ok" : "fail")}>
                  <i className="dot" />
                  <span>
                    {fmtTime(p.last.ts)} {p.last.detail}
                  </span>
                </div>
              )}
              <div className="actions">
                <button className="primary" data-testid="activate-btn" onClick={() => api.activateNow(p.id)}>激活</button>
                <button className="ghost" data-testid="edit-btn" onClick={() => setEditing(p)}>编辑</button>
                <button className="ghost danger" data-testid="delete-btn" onClick={() => setConfirmDel(p)}>删除</button>
              </div>
            </div>
          </div>
        ))}
        {!providers.length && (
          <div className="empty">
            <svg className="empty-mark" width="30" height="30" viewBox="0 0 24 24" fill="none" aria-hidden>
              <path
                d="M12.2 2c.4 3.4-.9 5.2-2.6 7C7.9 10.8 6 12.8 6 15.7a6 6 0 0 0 12 0c0-2.3-1-4.1-2.3-5.7-.6 1.1-1.4 1.9-2.4 2.3.4-3.5-.2-7-1.1-10.3z"
                fill="currentColor"
              />
            </svg>
            <div className="empty-title">还没有供应商</div>
            <div className="empty-sub">
              从 cc-switch 一键导入已有的 Coding Plan 配置，或手动添加一个供应商。
            </div>
            <div className="empty-actions">
              <button className="primary" onClick={onSync}>同步 cc-switch 配置</button>
              <button onClick={() => setEditing(null)}>＋ 手动添加</button>
            </div>
          </div>
        )}
      </div>

      <div className="logs">
        <h3>活动日志</h3>
        <div className="log-list" data-testid="log-list">
          {logs.map((l, i) => (
            <div key={i} className={"log " + (l.ok ? "ok" : "fail")}>
              <span className="log-time">{fmtTime(l.ts)}</span>
              <span className="log-name">{l.provider_name || "-"}</span>
              <span className="log-kind">{l.kind}</span>
              <span className="log-detail">{l.detail}</span>
            </div>
          ))}
          {!logs.length && <div className="log-empty">暂无日志</div>}
        </div>
      </div>

      {/* 删除确认（应用内弹窗，替代浏览器原生 confirm） */}
      {confirmDel && (
        <div className="modal-mask" onClick={() => setConfirmDel(null)}>
          <div className="modal confirm" onClick={(e) => e.stopPropagation()} data-testid="confirm-dialog">
            <h3>删除供应商</h3>
            <p>
              确定删除「<b>{confirmDel.name}</b>」？其定时设置与额度缓存会一并清除，该操作不可恢复。
            </p>
            <div className="form-actions">
              <button data-testid="confirm-cancel" onClick={() => setConfirmDel(null)}>取消</button>
              <button
                className="danger-solid"
                data-testid="confirm-ok"
                onClick={() => {
                  api.deleteProvider(confirmDel.id);
                  setConfirmDel(null);
                }}
              >
                删除
              </button>
            </div>
          </div>
        </div>
      )}

      {editing !== undefined && (
        <ProviderForm
          key={editing?.id || "new"}
          initial={editing}
          onClose={() => setEditing(undefined)}
          onSaved={() => {
            setEditing(undefined);
            refresh();
          }}
        />
      )}
    </div>
  );
}
