// 抓 README 用的界面截图：先查一轮额度让卡片有数据，再分别抓深色与浅色主题。
import { chromium } from "playwright-core";

const browser = await chromium.connectOverCDP("http://127.0.0.1:9222");
const page = browser
  .contexts()[0]
  .pages()
  .find(p => p.url().includes("localhost:1420"));
if (!page) throw new Error("找不到应用页面");

// 视口放大一点，两列卡片网格更出效果
await page.setViewportSize({ width: 1240, height: 820 });

// 主题模式先记下来，收尾还原
const modeBefore = await page.evaluate(() => localStorage.getItem("qs-theme"));

// 查一轮额度（真实数据；个别卡片报错也是真实状态）
await page.getByTestId("refresh-all-btn").click();

// 等主卡片（智谱）出档位、查询时间落定
await page
  .locator('[data-testid="provider-card"][data-name="智谱"] .tier')
  .first()
  .waitFor({ timeout: 30000 });
await page.waitForTimeout(4000); // 其余卡片陆续落定，进度条动画走完

// 等顶部的临时提示条自动消失（5 秒），别让它出现在截图里
await page.locator(".notice").waitFor({ state: "detached", timeout: 10000 }).catch(() => {});
await page.waitForTimeout(200);

const setTheme = async mode => {
  await page.evaluate(m => {
    localStorage.setItem("qs-theme", m);
    document.documentElement.dataset.theme = m;
  }, mode);
  await page.waitForTimeout(400); // 等过渡动画结束
};

await setTheme("dark");
await page.screenshot({ path: "docs/screenshot-dark.png" });
console.log("dark ✓");

await setTheme("light");
await page.screenshot({ path: "docs/screenshot-light.png" });
console.log("light ✓");

// 还原主题模式
if (modeBefore && ["auto", "dark", "light"].includes(modeBefore)) {
  await page.evaluate(m => {
    localStorage.setItem("qs-theme", m);
    const mq = window.matchMedia("(prefers-color-scheme: light)");
    document.documentElement.dataset.theme =
      m === "auto" ? (mq.matches ? "light" : "dark") : m;
  }, modeBefore);
}

await browser.close();
console.log("截图完成");
