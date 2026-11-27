import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface QuotaTier {
  name: string;
  utilization: number;
  resets_at: string | null;
  /** 重置时间 epoch 秒（与 resets_at 同源），前端算「剩余多久重置」用 */
  resets_at_ts: number | null;
  amount: string | null;
}

export interface QuotaResult {
  ok: boolean;
  plan: string | null;
  tiers: QuotaTier[];
  balance_text: string | null;
  error: string | null;
  ts: number;
}

export interface LogEntry {
  ts: number;
  provider_id: string;
  provider_name: string;
  kind: string;
  ok: boolean;
  detail: string;
}

export interface ProviderConfig {
  id: string;
  name: string;
  base_url: string;
  api_key: string;
  model: string;
  format: string;
  enabled: boolean;
  times: string[];
  cc_provider_id: string | null;
  cc_app_type: string | null;
  usage_url: string | null;
  access_key_id: string | null;
  secret_access_key: string | null;
  plan_type: string | null;
  team_organization_id: string | null;
  team_project_id: string | null;
}

export type ProviderCard = ProviderConfig & {
  quota: QuotaResult | null;
  last: LogEntry | null;
  /** 是否支持自动查额度（后端按域名/智谱团队标识判定） */
  supports_quota: boolean;
};

export interface SyncReport {
  imported: number;
  updated: number;
  skipped: number;
  total: number;
}

export const api = {
  getProviders: () => invoke<ProviderCard[]>("get_providers"),
  getLogs: () => invoke<LogEntry[]>("get_logs"),
  syncFromCc: () => invoke<SyncReport>("sync_from_cc"),
  saveProvider: (provider: ProviderConfig) =>
    invoke<ProviderConfig>("save_provider", { provider }),
  deleteProvider: (id: string) => invoke<void>("delete_provider", { id }),
  reorderProviders: (ids: string[]) => invoke<void>("reorder_providers", { ids }),
  setSchedule: (id: string, enabled: boolean, times: string[]) =>
    invoke<void>("set_schedule", { id, enabled, times }),
  activateNow: (id?: string) => invoke<number>("activate_now", { id: id ?? null }),
  queryQuota: (id: string) => invoke<void>("query_quota", { id }),
  getAutostart: () => invoke<boolean>("get_autostart"),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  /** 下载更新前调用：写「刚更新完」标记，更新重启后即使带 --minimized 也显示主窗口 */
  markJustUpdated: () => invoke<void>("mark_just_updated"),
  onChanged: (cb: () => void) => listen("state-changed", cb),
};
