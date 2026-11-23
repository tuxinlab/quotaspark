import { useEffect, useRef, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";

/** 常驻期间的复查间隔。应用开机自启、长期留在托盘里，只在启动时查一次的话，
 *  不重启就收不到新版本提示。 */
const RECHECK_MS = 6 * 60 * 60 * 1000;

type Phase = "idle" | "checking" | "available" | "downloading" | "installing";

/**
 * 顶栏「检查更新」按钮：状态直接体现在按钮文案上
 * （检查中… → 下载并安装 vX → 下载中 n% → 正在安装…），详细信息走通知条。
 *
 * 检查时机：启动时静默查一次，之后每 6 小时复查一次；手动点击随时可查。
 * 发现新版本只把按钮变成「下载并安装 vX」并提示一次，不会自动安装；
 * 检查失败（无更新源 / 离线）静默忽略。
 *
 * 安装流程（Windows）：下载安装包 → 插件用公钥验签 → 启动 NSIS 安装器，随后插件
 * 直接结束本进程（tauri-plugin-updater 的 install_inner 末尾是 std::process::exit），
 * 安装器再关闭残留进程、覆盖安装并自动重新打开新版本。所以安装成功时
 * `downloadAndInstall` 的 Promise 不会返回，「重启」这一步由安装器完成，
 * 前端没有、也不需要自己的重启动作。
 * 端点与公钥配置在 src-tauri/tauri.conf.json 的 plugins.updater。
 */
export default function UpdateButton({ onNotice }: { onNotice: (msg: string) => void }) {
  const [phase, setPhaseState] = useState<Phase>("idle");
  const [version, setVersion] = useState("");
  const [percent, setPercent] = useState<number | null>(null);
  const updateRef = useRef<Update | null>(null);
  // 定时器回调与异步流程里读不到最新的 state / props，用 ref 转一手
  const phaseRef = useRef<Phase>("idle");
  const noticeRef = useRef(onNotice);
  // 并发的检查共用同一个请求，避免重复访问更新源、拿到两个更新对象
  const inFlightRef = useRef<Promise<Update | null> | null>(null);

  useEffect(() => {
    noticeRef.current = onNotice;
  }, [onNotice]);

  const setPhase = (p: Phase) => {
    phaseRef.current = p;
    setPhaseState(p);
  };

  const busy = () => phaseRef.current !== "idle" && phaseRef.current !== "available";

  const query = () => {
    if (!inFlightRef.current) {
      inFlightRef.current = check().finally(() => {
        inFlightRef.current = null;
      });
    }
    return inFlightRef.current;
  };

  /** 采用检查得到的更新对象。同一个版本不重复提示；换下来的旧对象要释放，
   *  它持有后端资源。 */
  const adopt = async (found: Update, note: string | null) => {
    const known = updateRef.current;
    if (known === found) return;
    if (known?.version === found.version) {
      await found.close().catch(() => {});
      return;
    }
    if (known) await known.close().catch(() => {});
    updateRef.current = found;
    setVersion(found.version);
    setPhase("available");
    if (note) noticeRef.current(note);
  };

  // 手动检查（只在 idle 时可点）：有反馈，发现新版本时带上更新说明
  const manualCheck = async () => {
    setPhase("checking");
    try {
      const found = await query();
      if (!found) {
        setPhase("idle");
        noticeRef.current("已是最新版本");
        return;
      }
      await adopt(found, `发现新版本 v${found.version}${found.body ? `：${found.body}` : ""}`);
    } catch (e) {
      setPhase("idle");
      noticeRef.current(`检查更新失败：${e}（开发模式或更新源未配置时属预期）`);
    }
  };

  // 定时复查：不改按钮文案、失败静默，发现没提示过的版本才通知
  const recheck = async () => {
    if (busy()) return;
    try {
      const found = await query();
      if (found) {
        await adopt(found, `发现新版本 v${found.version}，点顶栏按钮即可安装`);
      }
    } catch {
      // 离线 / 更新源不可达：保持现状，下个周期再试
    }
  };

  const downloadAndInstall = async () => {
    const u = updateRef.current;
    if (!u) return;
    setPercent(null);
    setPhase("downloading");
    let total = 0;
    let received = 0;
    try {
      await u.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? 0;
            break;
          case "Progress":
            received += event.data.chunkLength;
            if (total > 0) {
              setPercent(Math.min(100, Math.round((received / total) * 100)));
            }
            break;
          case "Finished":
            // 这个事件在验签之前发出，只是下载完成：插件接着验签、启动安装器
            setPhase("installing");
            break;
        }
      });
      // 成功时走不到这里（见文件头）。万一进程还在，安装器也会自己关闭并重新
      // 打开应用，这里不需要、也不应该再自行重启。
    } catch (e) {
      // 更新对象仍然有效，回到「下载并安装」让用户可以重试
      setPhase("available");
      noticeRef.current(`下载或安装失败：${e}`);
    }
  };

  const label =
    phase === "checking"
      ? "检查中…"
      : phase === "available"
        ? `下载并安装 v${version}`
        : phase === "downloading"
          ? percent === null
            ? "下载中…"
            : `下载中 ${percent}%`
          : phase === "installing"
            ? "正在安装…"
            : "检查更新";

  // 启动时静默检查一次，之后定时复查。组件卸载时清掉定时器。
  useEffect(() => {
    void recheck();
    const id = window.setInterval(() => void recheck(), RECHECK_MS);
    return () => window.clearInterval(id);
    // 流程函数只通过 ref 与 setState 读写状态，首次渲染的闭包一直有效
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <button
      className={phase === "available" ? "primary" : ""}
      disabled={phase === "checking" || phase === "downloading" || phase === "installing"}
      title={phase === "available" ? `下载并安装 v${version}` : "检查并安装新版本"}
      data-testid="check-update-btn"
      onClick={phase === "available" ? downloadAndInstall : phase === "idle" ? manualCheck : undefined}
    >
      {label}
    </button>
  );
}
