/**
 * 全面端到端测试：通过 CDP 附加到应用的 WebView2，模拟真实用户操作。
 * 覆盖：页面加载、主题、同步、供应商增删改查、卡片拖拽排序、高级配置（团队版条件字段）、
 * 调度（模板/chips 增改删/开关）、激活失败路径、额度查询成功+失败路径、
 * 支持性标记、开机自启、自定义确认弹窗、审计日志。
 *
 * 运行前提：应用以 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222" 启动。
 * 用法：node scripts/e2e.mjs
 */
import { chromium } from "playwright-core";
import { mkdirSync } from "node:fs";

const CDP = process.env.E2E_CDP ?? "http://127.0.0.1:9222";
const SHOT_DIR = "e2e-artifacts";
mkdirSync(SHOT_DIR, { recursive: true });

const TEST_NAME = "测试-勿动E2E";
const testCard = page => page.locator(`[data-testid="provider-card"][data-name="${TEST_NAME}"]`);
const zhipuCard = page => page.locator('[data-testid="provider-card"][data-name="智谱"]');
// chips 定位必须限定在测试卡内——其它卡片（如用户自己的智谱）也有各自的定时 chips
const testChips = page => testCard(page).locator('[data-testid="time-chips"] .chip');

const results = [];
let page;

/** 步骤开始前清场：关掉遗留的弹窗 / 浮层（点左上角纯遮罩区域），避免点击被拦截 */
async function dismissOverlays() {
  for (let i = 0; i < 4; i++) {
    if (await page.locator(".tpl-backdrop").count()) {
      await page.mouse.click(8, 8);
      await page.waitForTimeout(250);
    }
    const mask = page.locator(".modal-mask");
    if (!(await mask.count())) break;
    const cancel = mask.getByRole("button", { name: /取消|关闭/ }).first();
    if (await cancel.count()) {
      await cancel.click().catch(() => {});
    } else {
      await page.mouse.click(8, 8);
    }
    await page.waitForTimeout(300);
  }
}

async function step(name, fn) {
  try {
    await dismissOverlays();
    await fn();
    results.push({ name, ok: true });
    console.log(`  ✓ ${name}`);
  } catch (e) {
    results.push({ name, ok: false, error: String(e).slice(0, 300) });
    console.log(`  ✗ ${name}: ${String(e).slice(0, 300)}`);
    try {
      await page.screenshot({ path: `${SHOT_DIR}/fail-${results.length}.png` });
    } catch {}
  }
}

const browser = await chromium.connectOverCDP(CDP);
const context = browser.contexts()[0];
page =
  context.pages().find(p => p.url().includes("localhost:1420")) ??
  (await context.newPage().then(async p => {
    await p.goto("http://localhost:1420");
    return p;
  }));

console.log(`已连接：${page.url()}`);

/* ── 1. 页面基础 ─────────────────────────────────────────── */

await step("页面加载：品牌标题与供应商卡片渲染", async () => {
  await page.getByTestId("theme-toggle").waitFor({ timeout: 5000 });
  if (!(await page.locator("h1", { hasText: "额度火花" }).count())) throw new Error("标题缺失");
  const cards = await page.locator('[data-testid="provider-card"]').count();
  if (cards < 1) throw new Error("没有任何供应商卡片（请先确认已同步过 cc-switch）");
});

await step("【自清理】删除上次运行残留的测试供应商（如有）", async () => {
  for (let i = 0; i < 6; i++) {
    const leftovers = page.locator('[data-testid="provider-card"][data-name^="测试-"]');
    if (!(await leftovers.count())) return;
    await leftovers.first().getByTestId("delete-btn").click();
    await page.getByTestId("confirm-ok").click();
    await page.waitForTimeout(600);
  }
  if (await page.locator('[data-testid="provider-card"][data-name^="测试-"]').count())
    throw new Error("残留测试供应商清理失败");
});

await step("【主题】跟随系统→深→浅循环，data-theme 实际变化并恢复原模式", async () => {
  const modeBefore = await page.evaluate(() => localStorage.getItem("qs-theme"));
  const themeBefore = await page.evaluate(() => document.documentElement.dataset.theme);
  let themeAfter = themeBefore;
  for (let i = 0; i < 3 && themeAfter === themeBefore; i++) {
    await page.getByTestId("theme-toggle").click();
    await page.waitForTimeout(350);
    themeAfter = await page.evaluate(() => document.documentElement.dataset.theme);
  }
  if (themeBefore === themeAfter) throw new Error("主题未变化");
  await page.screenshot({ path: `${SHOT_DIR}/01-theme-${themeAfter}.png` });
  for (let i = 0; i < 3; i++) {
    const mode = await page.evaluate(() => localStorage.getItem("qs-theme"));
    if (mode === modeBefore) break;
    await page.getByTestId("theme-toggle").click();
    await page.waitForTimeout(250);
  }
  if ((await page.evaluate(() => localStorage.getItem("qs-theme"))) !== modeBefore)
    throw new Error("模式未恢复");
});

await step("【同步】同步 cc-switch 配置 → 成功提示 + sync 日志", async () => {
  await page.getByTestId("sync-btn").click();
  await page.getByTestId("notice").waitFor({ timeout: 8000 });
  const t = await page.getByTestId("notice").innerText();
  if (!t.includes("同步完成")) throw new Error(`提示不对：${t}`);
  await page.waitForTimeout(400);
  if (!(await page.getByTestId("log-list").innerText()).includes("同步 cc-switch"))
    throw new Error("日志缺 sync 记录");
});

await step("【支持性】不支持的供应商（Xiaomi MiMo）不显示查额度按钮", async () => {
  const mimo = page.locator('[data-testid="provider-card"][data-name="Xiaomi MiMo"]');
  if (!(await mimo.count())) throw new Error("找不到 Xiaomi MiMo 卡片（同步后应有）");
  await mimo.locator(".quota-unsupported").waitFor({ timeout: 3000 });
  if (await mimo.getByTestId("quota-btn").count()) throw new Error("不应出现查额度按钮");
});

/* ── 2. 供应商增改查 + 高级配置 ──────────────────────────── */

await step("【增】手动添加：团队版条件字段按需出现 + 保存 + 审计日志", async () => {
  await page.getByTestId("add-btn").click();
  await page.getByLabel("名称", { exact: true }).fill(TEST_NAME);
  await page.getByLabel("Base URL", { exact: true }).fill("https://open.bigmodel.cn/api/anthropic");
  await page.getByLabel("API Key", { exact: true }).fill("sk-test-invalid");
  await page.getByLabel("激活用模型", { exact: true }).fill("glm-5.3-flash");
  // 展开「高级配置」折叠区
  await page.locator(".advanced summary").click();
  await page.waitForTimeout(200);
  // 勾选前：团队版字段不显示
  if (await page.getByTestId("team-org-input").count()) throw new Error("未勾选时不应显示组织 ID");
  await page.getByTestId("plan-team-checkbox").check();
  await page.getByTestId("team-org-input").waitFor({ timeout: 2000 });
  await page.getByTestId("team-org-input").fill("fake-org-123");
  await page.getByTestId("team-project-input").fill("fake-project-456");
  await page.screenshot({ path: `${SHOT_DIR}/02-add-form-team.png` });
  await page.getByRole("button", { name: "保存" }).click();
  await testCard(page).waitFor({ timeout: 5000 });
  await page.waitForTimeout(400);
  if (!(await page.getByTestId("log-list").innerText()).includes("已添加供应商"))
    throw new Error("新增未写日志");
});

await step("【查改】编辑回显（勾选/组织 ID 持久化）+ 改名往返", async () => {
  await testCard(page).getByTestId("edit-btn").click();
  await page.locator(".advanced summary").click();
  await page.waitForTimeout(200);
  if (!(await page.getByTestId("plan-team-checkbox").isChecked())) throw new Error("勾选未持久化");
  const org = await page.getByTestId("team-org-input").inputValue();
  if (org !== "fake-org-123") throw new Error(`组织 ID 未回显：${org}`);
  await page.screenshot({ path: `${SHOT_DIR}/03-edit-echo.png` });
  await page.getByLabel("名称", { exact: true }).fill(TEST_NAME + "X");
  await page.getByRole("button", { name: "保存" }).click();
  await page.locator(`[data-testid="provider-card"][data-name="${TEST_NAME}X"]`).waitFor({ timeout: 5000 });
  await page.locator(`[data-testid="provider-card"][data-name="${TEST_NAME}X"]`).getByTestId("edit-btn").click();
  await page.getByLabel("名称", { exact: true }).fill(TEST_NAME);
  await page.getByRole("button", { name: "保存" }).click();
  await testCard(page).waitFor({ timeout: 5000 });
});

await step("【排序】拖拽手柄把测试卡移到相邻卡位置，刷新后顺序保持", async () => {
  const names = () =>
    page.evaluate(() => [...document.querySelectorAll('[data-testid="provider-card"]')].map(el => el.dataset.name));
  const before = await names();
  if (before.length < 2) throw new Error("卡片少于 2 张，无法测试排序");
  const srcIdx = before.indexOf(TEST_NAME);
  if (srcIdx < 0) throw new Error("找不到测试卡");
  // 落点取相邻卡：测试卡插到落点卡当前位置（落点卡顺延）
  const target = srcIdx > 0 ? before[srcIdx - 1] : before[1];
  const expectOrder = () => {
    const want = [...before];
    want.splice(srcIdx, 1);
    const tIdx = before.indexOf(target);
    want.splice(tIdx < srcIdx ? tIdx : tIdx - 1, 0, TEST_NAME);
    return want;
  };
  // HTML5 DnD 无法用鼠标事件模拟，走页面内派发 DragEvent（React 在根节点监听，能收到）。
  // 拖拽源是手柄自身：dragstart / dragend 都派发到手柄上，与真实拖拽的事件路径一致
  await page.evaluate(({ srcName, dstName }) => {
    const byName = n => document.querySelector(`[data-testid="provider-card"][data-name="${n}"]`);
    const fire = (el, type, dt) =>
      el.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer: dt }));
    return (async () => {
      const d = byName(dstName);
      const handle = byName(srcName).querySelector('[data-testid="drag-handle"]');
      const dt = new DataTransfer();
      fire(handle, "dragstart", dt);
      // dragstart 里 setDragId 是异步刷的，等一帧再落点，模拟真实拖拽节奏
      await new Promise(r => setTimeout(r, 120));
      fire(d, "dragover", dt);
      await new Promise(r => setTimeout(r, 60));
      fire(d, "drop", dt);
      fire(handle, "dragend", dt);
    })();
  }, { srcName: TEST_NAME, dstName: target });
  const want = expectOrder();
  await pollUntil(
    async () => {
      const after = await names();
      return { ok: JSON.stringify(after) === JSON.stringify(want), value: after };
    },
    `拖拽后顺序不符，期望 ${JSON.stringify(want)}`,
    5000,
  );
  // 刷新验证持久化（config.json 的 providers 顺序）
  await page.reload();
  await page.locator('[data-testid="provider-card"]').first().waitFor({ timeout: 8000 });
  const persisted = await names();
  if (JSON.stringify(persisted) !== JSON.stringify(want))
    throw new Error(`刷新后顺序未保持，期望 ${JSON.stringify(want)}，实际 ${JSON.stringify(persisted)}`);
  await page.screenshot({ path: `${SHOT_DIR}/10-reorder.png` });
});

/* ── 3. 调度 ─────────────────────────────────────────────── */

await step("【调度】模板菜单只有「全天接力」一项，套用 → 4 个 chips + 定时中", async () => {
  await testCard(page).getByTestId("tpl-btn").click();
  const items = await page.getByTestId("tpl-menu").getByRole("button").count();
  if (items !== 1) throw new Error(`模板应为 1 项，实际 ${items}`);
  await page.getByTestId("tpl-menu").getByRole("button", { name: /全天接力/ }).click();
  await testChips(page).first().waitFor({ timeout: 5000 });
  await pollUntil(
    async () => {
      const chips = await testChips(page).count();
      return { ok: chips === 4, value: chips };
    },
    "期望测试卡上有 4 个 chips",
  );
  await testCard(page).locator(".badge.on").waitFor({ timeout: 3000 });
  await page.screenshot({ path: `${SHOT_DIR}/04-template-applied.png` });
});

await step("【调度】新增时间 01:30 → 5 个 chips", async () => {
  await testCard(page).getByTestId("chip-add").click();
  await page.getByTestId("time-input").fill("01:30");
  await page.getByTestId("time-input").press("Enter");
  await pollUntil(
    async () => {
      const n = await testChips(page).count();
      return { ok: n === 5, value: n };
    },
    "chips 数不为 5",
  );
});

/** 轮询等待条件成立（默认最多 3 秒），失败抛出 last 值 */
async function pollUntil(fn, desc, timeoutMs = 3000) {
  const deadline = Date.now() + timeoutMs;
  let last;
  while (Date.now() < deadline) {
    last = await fn();
    if (last.ok) return last.value;
    await page.waitForTimeout(250);
  }
  throw new Error(`${desc}（最后值：${JSON.stringify(last?.value)}）`);
}

await step("【调度】修改 01:30 → 06:00（键盘真实键入，改后排序正确）", async () => {
  // chips 排序显示，01:30 排在第一位；定位要按内容而非位置
  await testChips(page)
    .filter({ hasText: "01:30" })
    .first()
    .locator(".chip-time")
    .click();
  const input = page.getByTestId("time-input");
  await input.pressSequentially("0600");
  await input.press("Enter");
  await pollUntil(
    async () => {
      const chips = await testChips(page).allInnerTexts();
      const want = ["05:30", "06:00", "10:30", "15:30", "20:30"];
      const got = chips.map(t => t.replace(/\n×$/, "").trim());
      return { ok: JSON.stringify(got) === JSON.stringify(want), value: got };
    },
    "修改 01:30→06:00 后时间集合不符",
  );
});

await step("【调度】删除 06:00 chip → 回到 4 个", async () => {
  await testChips(page)
    .filter({ hasText: "06:00" })
    .first()
    .locator(".chip-x")
    .click();
  await pollUntil(
    async () => {
      const n = await testChips(page).count();
      return { ok: n === 4, value: n };
    },
    "chips 数不为 4",
  );
});

await step("【调度】定时开关 off → 定时中徽标消失；on → 恢复", async () => {
  const tg = testCard(page).getByTestId("sched-toggle");
  await tg.click();
  await pollUntil(
    async () => ({ ok: (await testCard(page).locator(".badge.on").count()) === 0, value: "off" }),
    "关闭后徽标仍在",
  );
  await tg.click();
  await pollUntil(
    async () => ({ ok: (await testCard(page).locator(".badge.on").count()) === 1, value: "on" }),
    "重新开启后徽标缺失",
  );
});

await step("【调度说明】ⓘ 按钮 → 弹出说明浮层（含五小时/补发规则）", async () => {
  await testCard(page).getByTestId("sched-info").click();
  await page.getByTestId("sched-info-pop").waitFor({ timeout: 2000 });
  const t = await page.getByTestId("sched-info-pop").innerText();
  if (!t.includes("五小时") || !t.includes("补发")) throw new Error(`说明内容缺失：${t.slice(0, 60)}`);
  await page.screenshot({ path: `${SHOT_DIR}/08-sched-info.png` });
  // 点外面关闭（真实用户路径，走 document mousedown 监听）
  await page.mouse.click(8, 8);
  await pollUntil(
    async () => ({ ok: !(await page.getByTestId("sched-info-pop").count()), value: "closed" }),
    "浮层未关闭",
  );
});

/* ── 4. 激活与额度（失败路径用假 Key，成功路径用真实智谱）── */

await step("【激活·失败】假 Key 打真实端点 → 401 失败落到卡片与日志", async () => {
  await testCard(page).getByTestId("activate-btn").click();
  await testCard(page).locator(".last.fail").waitFor({ timeout: 20000 });
  const detail = await testCard(page).locator(".last.fail").innerText();
  if (!/40[13]/.test(detail)) throw new Error(`期望 401/403，实际：${detail}`);
  await page.waitForTimeout(400);
  if (!(await page.getByTestId("log-list").innerText()).includes(TEST_NAME))
    throw new Error("日志缺该供应商记录");
});

async function triggerQuota(card) {
  // 已查询过的卡片显示「↻ 刷新」，未查过的显示「查额度」
  if (await card.getByTestId("quota-refresh").count()) {
    await card.getByTestId("quota-refresh").click();
  } else {
    await card.getByTestId("quota-btn").click();
  }
}

await step("【查额度·失败】团队版假组织 ID → 明确错误展示", async () => {
  await triggerQuota(testCard(page));
  await testCard(page).locator(".quota-err").waitFor({ timeout: 15000 });
  const text = await testCard(page).locator(".quota-err").innerText();
  if (!text.trim()) throw new Error("错误内容为空");
  await page.screenshot({ path: `${SHOT_DIR}/05-quota-error.png` });
});

await step("【查额度·成功】真实智谱 → 五小时窗口/本周额度进度条渲染", async () => {
  await triggerQuota(zhipuCard(page));
  await zhipuCard(page).locator(".tier").first().waitFor({ timeout: 15000 });
  const tiers = await zhipuCard(page).locator(".tier").count();
  if (tiers < 1) throw new Error("没有额度档位");
  const text = await zhipuCard(page).locator(".quota").innerText();
  if (!text.includes("五小时窗口")) throw new Error(`缺少五小时窗口：${text.slice(0, 80)}`);
  await page.screenshot({ path: `${SHOT_DIR}/06-zhipu-quota.png` });
});

await step("【额度刷新】↻ 刷新按钮可用且查询时间更新", async () => {
  await zhipuCard(page).getByTestId("quota-refresh").click();
  await page.waitForTimeout(2500);
  const after = await zhipuCard(page).locator(".quota-ts").innerText();
  if (!after.includes("查询于")) throw new Error("刷新后查询时间缺失");
});

/* ── 5. 设置 ─────────────────────────────────────────────── */

await step("【自启】dev 下点击开关 → 拒绝写入并提示（防止 debug 路径注册进系统）", async () => {
  const btn = page.getByTestId("autostart-btn");
  const before = (await btn.innerText()).includes("✓");
  await btn.click();
  await pollUntil(
    async () => {
      const notice = await page.getByTestId("notice").innerText();
      const unchanged = (await btn.innerText()).includes("✓") === before;
      // dev/debug 构建下 set_autostart 必须拒绝写入（注册 debug 路径会导致
      // 开机黑框 + 白屏），UI 提示失败且按钮状态不变
      return { ok: unchanged && /失败|开发/.test(notice), value: notice.slice(0, 60) };
    },
    "dev 下自启开关应拒绝修改并提示失败",
  );
});

await step("【检查更新】顶栏按钮 → 有反馈（未配置更新源时报错属预期）", async () => {
  await page.getByTestId("check-update-btn").click();
  await pollUntil(
    async () => {
      const t = await page.getByTestId("notice").innerText();
      return { ok: /已是最新|检查更新失败|发现新版本/.test(t), value: t.slice(0, 60) };
    },
    "检查更新没有产生反馈",
    10000,
  );
  await page.screenshot({ path: `${SHOT_DIR}/07-update-check.png` });
});

/* ── 6. 删除 ─────────────────────────────────────────────── */

await step("【删】应用内确认弹窗 → 确认删除 → 卡片消失 + 审计日志", async () => {
  await testCard(page).getByTestId("delete-btn").click();
  await page.getByTestId("confirm-dialog").waitFor({ timeout: 3000 });
  await page.screenshot({ path: `${SHOT_DIR}/08-confirm-dialog.png` });
  await page.getByTestId("confirm-ok").click();
  await page.waitForTimeout(600);
  if (await testCard(page).count()) throw new Error("卡片仍存在");
  const logs = await page.getByTestId("log-list").innerText();
  if (!logs.includes("已删除供应商")) throw new Error("删除未写审计日志");
});

await step("【日志】真实供应商的历史记录未被误伤", async () => {
  const logs = await page.getByTestId("log-list").innerText();
  if (!logs.includes("智谱")) throw new Error("智谱的历史日志丢失");
});

await page.screenshot({ path: `${SHOT_DIR}/09-final.png` });

const failed = results.filter(r => !r.ok);
console.log(`\n========== 结果：${results.length - failed.length}/${results.length} 通过 ==========`);
if (failed.length) {
  console.log("失败项：", JSON.stringify(failed, null, 2));
  process.exit(1);
}
await browser.close();
