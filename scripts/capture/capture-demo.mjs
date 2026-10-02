// 给官网录客户端的截图和录屏：客户端的真实界面（crates/meshora-desktop/ui）加假后端（dev/mock.js），
// 用 Playwright 的 Chromium 打开各个场景，截图转成 WebP、录屏存成 WebM，放进 docs/public/demo/。
//
// 本机跑，不进 CI：界面改了就重录一遍，产物提交进仓库。
//
//   cd scripts/capture && npm ci && npx playwright install chromium && npm run capture
//
// 无头 Chromium 没有显卡，Glassium 用软件 WebGL（SwiftShader）画，慢一点但画得出来。

import { createServer } from "node:http";
import { readFile, mkdir, rename, rm, stat, readdir } from "node:fs/promises";
import { extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import sharp from "sharp";

const here = fileURLToPath(new URL(".", import.meta.url));
const root = resolve(here, "../..");
const site = join(root, "crates/meshora-desktop");
const out = join(root, "docs/public/demo");
const tmp = join(here, ".videos");

const DESKTOP = { width: 1100, height: 720 };
const PHONE = { width: 390, height: 844 };
const ANDROID_UA =
  "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Mobile Safari/537.36";

// ---------- 本地静态服务器：只放 crates/meshora-desktop 下的文件 ----------

const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".png": "image/png", ".woff2": "font/woff2", ".wasm": "application/wasm", ".json": "application/json" };
const server = createServer(async (req, res) => {
  const path = decodeURIComponent(new URL(req.url, "http://x").pathname);
  const file = resolve(site, `.${path}`);
  if (!file.startsWith(site)) return res.writeHead(403).end();
  try {
    const body = await readFile(file);
    res.writeHead(200, { "content-type": TYPES[extname(file)] || "application/octet-stream", "cache-control": "no-store" });
    res.end(body);
  } catch {
    res.writeHead(404).end();
  }
});
await new Promise((ok) => server.listen(0, "127.0.0.1", ok));
// 客户端左下角显示的版本号：和 Cargo.toml 的工作区版本一致
const version = (await readFile(join(root, "Cargo.toml"), "utf8")).match(/^version = "([^"]+)"/m)[1];
const base = `http://127.0.0.1:${server.address().port}/dev/index.html`;

// ---------- 浏览器 ----------

const browser = await chromium.launch({
  args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"],
});
await rm(tmp, { recursive: true, force: true });
await mkdir(out, { recursive: true });

const sleep = (ms) => new Promise((ok) => setTimeout(ok, ms));

async function context(kind, { video = false } = {}) {
  const phone = kind === "phone";
  const viewport = phone ? PHONE : DESKTOP;
  return browser.newContext({
    viewport,
    deviceScaleFactor: video ? 1 : 2,
    isMobile: phone,
    hasTouch: phone,
    userAgent: phone ? ANDROID_UA : undefined,
    colorScheme: "dark",
    recordVideo: video ? { dir: tmp, size: viewport } : undefined,
  });
}

async function open(ctx, query) {
  const page = await ctx.newPage();
  page.on("pageerror", (err) => console.warn(`  页面出错（${query}）：${err.message}`));
  await page.goto(`${base}?${query}&v=${version}`);
  return page;
}

/** 点一个文字是 `text` 的按钮 */
const click = (page, text) => page.locator("button", { hasText: text }).first().click();

/** 截一张图，转成 WebP（宽度压到 `width`） */
async function shot(page, name, width) {
  const png = await page.screenshot();
  await sharp(png).resize({ width }).webp({ quality: 82 }).toFile(join(out, `${name}.webp`));
  console.log(`  截图 ${name}`);
}

/** 录一段：`act` 在页面上做事，录完存成 name.webm */
async function record(kind, query, name, act) {
  const ctx = await context(kind, { video: true });
  const page = await open(ctx, query);
  await act(page);
  const video = page.video();
  await ctx.close();
  await rename(await video.path(), join(out, `${name}.webm`));
  console.log(`  录屏 ${name}`);
}

// ---------- 截图 ----------

console.log("截图");
{
  const ctx = await context("desktop");
  const settle = 1200;

  let page = await open(ctx, "s=connected&nointro=1");
  await sleep(2500);
  await shot(page, "network-desktop", 1600);
  await click(page, "设置");
  await sleep(settle);
  await shot(page, "settings-desktop", 1600);
  await page.close();

  page = await open(ctx, "s=join&nointro=1");
  await sleep(2000);
  await shot(page, "start-desktop", 1600);
  await page.close();

  page = await open(ctx, "s=direct-host&nointro=1");
  await sleep(2200);
  await shot(page, "direct-desktop", 1600);
  await click(page, "邀请朋友");
  await sleep(1800);
  await shot(page, "invite-desktop", 1600);
  await page.close();

  page = await open(ctx, "s=owner&nointro=1");
  await sleep(1800);
  await click(page, "管理");
  await sleep(settle + 600);
  await shot(page, "admin-desktop", 1600);
  await page.close();
  await ctx.close();

  const phone = await context("phone");
  page = await open(phone, "s=connected&nointro=1");
  await sleep(2500);
  await shot(page, "network-phone", 780);
  await click(page, "设置");
  await sleep(settle);
  await shot(page, "settings-phone", 780);
  await page.close();
  page = await open(phone, "s=direct-guest&nointro=1");
  await sleep(2200);
  await shot(page, "direct-phone", 780);
  await phone.close();
}

// ---------- 录屏 ----------

console.log("录屏");

// 启动：点阵扩散、画 Logo、飞到角落、界面浮上来
await record("desktop", "s=story-boot", "boot", async () => sleep(4200));

// 一起联机：贴网络码 → 连接中 → 连上 → 朋友一个个飞出来
await record("desktop", "s=story-connect&nointro=1", "connect", async (page) => {
  await sleep(1400);
  const code = "Bq7Zt4mN0xR2c8vL5kP1wY9sD3fG6hJ8aE2uQ4iO7tU=@play.example.com:7443/pZQ0bJbVv2u3Xy1a9cD8eF#3q2-7wEYkQ6n0Cf8Hs5VYA";
  await page.locator("textarea").first().fill(code);
  await sleep(500);
  await click(page, "加入");
  await sleep(7600);
});

// 直连打洞：光点往中间跑，3 秒后打通、线亮一下
await record("desktop", "s=story-punch&nointro=1", "punch", async () => sleep(6500));

// 手机：手指划过点阵、切页
await record("phone", "s=connected&nointro=1", "phone", async (page) => {
  await sleep(1600);
  const cdp = await page.context().newCDPSession(page);
  const touch = (type, x, y) => cdp.send("Input.dispatchTouchEvent", { type, touchPoints: type === "touchEnd" ? [] : [{ x, y }] });
  // 在空白处划一道弧线：点阵跟着手指聚拢
  await touch("touchStart", 60, 640);
  for (let i = 0; i <= 30; i++) {
    await touch("touchMove", 60 + i * 9, 640 - Math.sin((i / 30) * Math.PI) * 220);
    await sleep(40);
  }
  await touch("touchEnd", 0, 0);
  await sleep(900);
  await click(page, "朋友");
  await sleep(1500);
  await click(page, "设置");
  await sleep(1500);
  await click(page, "网络");
  await sleep(1400);
});

await browser.close();
server.close();
await rm(tmp, { recursive: true, force: true });

// ---------- 报大小 ----------

console.log("产物（docs/public/demo）");
let total = 0;
for (const file of (await readdir(out)).sort()) {
  const { size } = await stat(join(out, file));
  total += size;
  console.log(`  ${file.padEnd(24)} ${(size / 1024).toFixed(0).padStart(6)} KB`);
}
console.log(`  合计 ${(total / 1024 / 1024).toFixed(2)} MB`);
