import { useState } from "react";
import { api, type ProviderConfig } from "./api";

interface Props {
  initial: ProviderConfig | null;
  onClose: () => void;
  onSaved: () => void;
}

const BLANK: ProviderConfig = {
  id: "",
  name: "",
  base_url: "",
  api_key: "",
  model: "",
  format: "anthropic",
  enabled: false,
  times: [],
  cc_provider_id: null,
  cc_app_type: null,
  usage_url: null,
  access_key_id: null,
  secret_access_key: null,
  plan_type: null,
  team_organization_id: null,
  team_project_id: null,
};

/** 与 src-tauri/src/quota.rs detect_kind 的支持名单保持一致 */
export const QUOTA_SUPPORTED_HINT =
  "支持自动查额度：Kimi For Coding、智谱 GLM（个人版/团队版）、MiniMax、" +
  "OpenCode Go、Command Code、小米 MiMo Token Plan、Ollama Cloud、火山方舟 Agent/Coding Plan、ZenMux；" +
  "按量余额类：DeepSeek、StepFun、SiliconFlow、OpenRouter、Novita AI、小米 MiMo。" +
  "按 Base URL 的域名识别，其余域名暂不支持，激活不受影响。";

/** 与后端 quota.rs 的识别规则同效：主机名是该域名或它的子域（按标签边界），
 *  后面再跟路径前缀。中转站把官方域名写在路径里、仿冒域名都不算，
 *  否则表单显示了输入框、卡片却提示不支持查额度。 */
const hostPattern = (host: string, path = "") =>
  new RegExp(
    String.raw`^\s*(?:[a-z][a-z\d+.-]*://)?(?:[^/?#@]*@)?(?:[\w-]+\.)*${host}\.?(?::\d+)?` +
      (path || String.raw`(?=[/?#]|\s*$)`),
    "i",
  );
const VOLCENGINE_URL = hostPattern(String.raw`volces\.com`, String.raw`/api/(?:plan|coding)`);
const ZENMUX_URL = hostPattern(String.raw`zenmux\.ai`);

export default function ProviderForm({ initial, onClose, onSaved }: Props) {
  const [p, setP] = useState<ProviderConfig>(initial ?? BLANK);
  const [err, setErr] = useState("");

  const set = <K extends keyof ProviderConfig>(k: K, v: ProviderConfig[K]) =>
    setP((prev) => ({ ...prev, [k]: v }));

  const save = async () => {
    try {
      await api.saveProvider({ ...p, times: p.times.filter(Boolean) });
      onSaved();
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="modal-mask" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3>{initial ? "编辑供应商" : "手动添加供应商"}</h3>

        <label>
          名称
          <input value={p.name} onChange={(e) => set("name", e.target.value)} placeholder="智谱 GLM" />
        </label>

        <label>
          接口格式
          <select value={p.format} onChange={(e) => set("format", e.target.value)}>
            <option value="anthropic">Anthropic Messages（/v1/messages）</option>
            <option value="openai">OpenAI Chat Completions（/v1/chat/completions）</option>
          </select>
        </label>

        <label>
          Base URL
          <input
            value={p.base_url}
            onChange={(e) => set("base_url", e.target.value)}
            placeholder="https://open.bigmodel.cn/api/anthropic"
          />
        </label>

        <label>
          API Key
          <input
            type="password"
            value={p.api_key}
            onChange={(e) => set("api_key", e.target.value)}
            placeholder=" sk-..."
          />
        </label>

        <label>
          激活用模型
          <input
            value={p.model}
            onChange={(e) => set("model", e.target.value)}
            placeholder="glm-5.3"
          />
        </label>

        <label className="check">
          <input
            type="checkbox"
            checked={p.enabled}
            onChange={(e) => set("enabled", e.target.checked)}
          />
          启用定时激活（也可在卡片上的调度行里设置）
        </label>

        <details className="advanced">
          <summary>额度查询高级配置（智谱团队 / 火山方舟 / ZenMux 才需要）</summary>
          <p className="adv-hint">{QUOTA_SUPPORTED_HINT}</p>
          <p className="adv-hint">
            火山方舟 AK/SK、ZenMux 用量端点的输入框，会在上方 Base URL
            填入对应域名下的地址（火山的 volces.com/api/plan、/api/coding，或 zenmux.ai）后自动出现；
            智谱团队版勾选后填写组织/项目 ID；其余供应商查额度直接用上方 API Key，无需额外配置。
          </p>
          <label className="check">
            <input
              type="checkbox"
              checked={p.plan_type === "zhipu_team"}
              data-testid="plan-team-checkbox"
              onChange={(e) => set("plan_type", e.target.checked ? "zhipu_team" : null)}
            />
            智谱 GLM 团队版（与个人版同域名无法自动识别，勾选后请填写下方组织/项目 ID）
          </label>
          {p.plan_type === "zhipu_team" && (
            <>
              <label>
                智谱团队版 组织 ID
                <input
                  value={p.team_organization_id ?? ""}
                  onChange={(e) => set("team_organization_id", e.target.value)}
                  data-testid="team-org-input"
                />
              </label>
              <label>
                智谱团队版 项目 ID
                <input
                  value={p.team_project_id ?? ""}
                  onChange={(e) => set("team_project_id", e.target.value)}
                  data-testid="team-project-input"
                />
              </label>
            </>
          )}
          {VOLCENGINE_URL.test(p.base_url) && (
            <>
              <label>
                火山方舟 AccessKey ID
                <input
                  value={p.access_key_id ?? ""}
                  onChange={(e) => set("access_key_id", e.target.value)}
                  placeholder="与推理 API Key 是两套凭据"
                />
              </label>
              <label>
                火山方舟 Secret Access Key
                <input
                  type="password"
                  value={p.secret_access_key ?? ""}
                  onChange={(e) => set("secret_access_key", e.target.value)}
                />
              </label>
            </>
          )}
          {ZENMUX_URL.test(p.base_url) && (
            <label>
              ZenMux 用量端点 URL
              <input
                value={p.usage_url ?? ""}
                onChange={(e) => set("usage_url", e.target.value)}
                placeholder="如 https://zenmux.ai/api/usage"
              />
            </label>
          )}
        </details>

        {err && <div className="form-err">{err}</div>}

        <p className="hint">
          激活 = 向该供应商发送一条 max_tokens=1 的最小请求（内容 "hi"），HTTP 2xx
          即开始计算五小时窗口。Key 仅保存在本机 ~/.cc-activator/config.json，不会上传。
        </p>

        <div className="form-actions">
          <button onClick={onClose}>取消</button>
          <button className="primary" onClick={save}>保存</button>
        </div>
      </div>
    </div>
  );
}
