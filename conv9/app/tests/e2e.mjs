import assert from "node:assert/strict";
import { existsSync } from "node:fs";

import { chromium } from "playwright-core";
import { startPreviewServer } from "../preview-server.mjs";

const chromeExecutable =
  process.env.CHROME_BIN ||
  ["/usr/bin/google-chrome-stable", "/usr/bin/chromium", "/usr/bin/google-chrome"].find(
    (candidate) => existsSync(candidate),
  );

assert.ok(chromeExecutable, "Set CHROME_BIN to a Chrome or Chromium executable");

let server;
let browser;
const pageErrors = [];
const failedResponses = [];

try {
  server = await startPreviewServer({
    port: Number(process.env.CONV9_TEST_PORT || 0),
    quiet: true,
  });
  const address = server.address();
  assert.ok(address && typeof address === "object", "preview server address");
  const baseUrl = `http://127.0.0.1:${address.port}`;
  const rangeResponse = await fetch(
    `${baseUrl}/outputs/multiresolution/short/ambient_guitar__amazonian_dolphins.wav`,
    { headers: { Range: "bytes=1000-1099" } },
  );
  assert.equal(rangeResponse.status, 206, "preview server supports media ranges");
  assert.equal(rangeResponse.headers.get("accept-ranges"), "bytes");
  assert.equal((await rangeResponse.arrayBuffer()).byteLength, 100);

  browser = await chromium.launch({
    executablePath: chromeExecutable,
    headless: true,
    args: ["--no-sandbox", "--autoplay-policy=no-user-gesture-required"],
  });
  const page = await browser.newPage({ viewport: { width: 1280, height: 860 } });
  page.on("pageerror", (error) => pageErrors.push(error.message));
  page.on("response", (response) => {
    if (response.status() >= 400) {
      failedResponses.push(`${response.status()} ${response.url()}`);
    }
  });

  await page.goto(`${baseUrl}/app/src/`, { waitUntil: "domcontentloaded" });
  await expectText(page, "#matrixStatus", "792 renders");
  await page.waitForFunction(() => {
    const audio = document.querySelector("#audio");
    const waveform = document.querySelector("#waveform");
    const spectrum = document.querySelector("#spectrogram");
    return (
      audio?.readyState >= HTMLMediaElement.HAVE_METADATA &&
      waveform?.width >= 300 &&
      spectrum?.width >= 300 &&
      waveform?.getAttribute("aria-busy") === "false" &&
      spectrum?.getAttribute("aria-busy") === "false" &&
      document.querySelector("#renderTitle")?.textContent === "multi / short"
    );
  });

  assert.equal(await page.locator("#pairSelect option").count(), 66, "pair count");
  assert.equal(await page.locator("#algorithmButtons button").count(), 4, "algorithm count");
  assert.equal(await page.locator("#presetButtons button").count(), 3, "preset count");
  assert.equal(
    await page.locator("#algorithmButtons button[aria-pressed='true']").textContent(),
    "multi",
  );
  assert.equal(
    await page.locator("#presetButtons button[aria-pressed='true']").textContent(),
    "short",
  );
  await assertAudioReady(page, "/multiresolution/short/");
  await assertCanvasHasVariation(page, "#waveform");
  await assertCanvasHasVariation(page, "#spectrogram");
  await assertNoViewportOverflow(page);
  assert.equal(await page.locator("#errorPanel").isHidden(), true, "error panel hidden");

  await setAudioTime(page, 24);
  await page.getByRole("button", { name: "medium", exact: true }).click();
  await expectText(page, "#renderTitle", "multi / medium");
  await assertAudioReady(page, "/multiresolution/medium/");
  await expectAudioTime(page, 24, 0.5);

  await page.getByRole("button", { name: "Play" }).click();
  await page.waitForFunction(() => !document.querySelector("#audio").paused);
  const beforeSwitch = await audioTime(page);
  await page.getByRole("button", { name: "wola", exact: true }).click();
  await expectText(page, "#renderTitle", "wola / medium");
  await assertAudioReady(page, "/sliding_wola/medium/");
  await page.waitForFunction(() => !document.querySelector("#audio").paused);
  assert.ok(
    (await audioTime(page)) >= beforeSwitch - 0.5,
    "playing selection switch preserves position",
  );
  await page.getByRole("button", { name: "Pause" }).click();
  await page.waitForFunction(() => document.querySelector("#audio").paused);
  await page.locator("body").press("Space");
  await page.waitForFunction(() => !document.querySelector("#audio").paused);
  await page.locator("body").press("Space");
  await page.waitForFunction(() => document.querySelector("#audio").paused);

  await page.locator("#volume").evaluate((input) => {
    input.value = "0.37";
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  assert.equal(await page.locator("#audio").evaluate((audio) => audio.volume), 0.37);

  await page.locator("#seek").evaluate((input) => {
    input.value = "11";
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await expectAudioTime(page, 11, 0.2);
  await page.locator("body").press("ArrowRight");
  await expectAudioTime(page, 16, 0.2);
  await page.locator("body").press("ArrowLeft");
  await expectAudioTime(page, 11, 0.2);

  const waveform = page.locator("#waveform");
  const bounds = await waveform.boundingBox();
  assert.ok(bounds, "waveform is visible");
  await waveform.click({ position: { x: bounds.width * 0.75, y: bounds.height / 2 } });
  await expectAudioTime(page, 45, 0.5);

  const sourceBeforeResize = await audioSource(page);
  const timeBeforeResize = await audioTime(page);
  await page.setViewportSize({ width: 900, height: 640 });
  await page.waitForTimeout(500);
  assert.equal(await audioSource(page), sourceBeforeResize, "resize does not reload audio");
  await expectAudioTime(page, timeBeforeResize, 0.2);
  await assertCanvasHasVariation(page, "#waveform");
  await assertCanvasHasVariation(page, "#spectrogram");
  await assertNoViewportOverflow(page);

  const secondPair = await page.locator("#pairSelect option").nth(1).getAttribute("value");
  await page.locator("#pairSelect").selectOption(secondPair);
  await page.waitForFunction(
    (pair) => document.querySelector("#audio").dataset.path.includes(pair),
    secondPair,
  );
  await assertAudioReady(page, "/sliding_wola/medium/");

  await page.getByRole("button", { name: "short", exact: true }).click();
  await page.getByRole("button", { name: "long", exact: true }).click();
  await expectText(page, "#renderTitle", "wola / long");
  await assertAudioReady(page, "/sliding_wola/long/");
  await page.waitForFunction(
    () => document.querySelector("#errorPanel").hidden,
  );

  assert.deepEqual(pageErrors, [], `page errors: ${pageErrors.join("\n")}`);
  assert.deepEqual(
    failedResponses,
    [],
    `failed responses: ${failedResponses.join("\n")}`,
  );
  console.log("conv9 browser functional test passed");
} finally {
  await browser?.close();
  if (server) {
    await new Promise((resolveClose, rejectClose) => {
      server.close((error) => (error ? rejectClose(error) : resolveClose()));
    });
  }
}

async function expectText(page, selector, expected) {
  await page.waitForFunction(
    ({ target, text }) => document.querySelector(target)?.textContent === text,
    { target: selector, text: expected },
  );
}

async function assertAudioReady(page, pathPart) {
  await page.waitForFunction(
    (part) => {
      const audio = document.querySelector("#audio");
      const normalizedPart = part.replace(/^\/+/, "");
      return (
        audio.readyState >= HTMLMediaElement.HAVE_METADATA &&
        Math.abs(audio.duration - 60) < 0.01 &&
        audio.currentSrc.startsWith("blob:") &&
        audio.dataset.path.includes(normalizedPart)
      );
    },
    pathPart,
  );
}

async function assertCanvasHasVariation(page, selector) {
  await page.waitForFunction((target) => {
    const canvas = document.querySelector(target);
    if (!canvas || canvas.width < 2 || canvas.height < 2) return false;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    const data = context.getImageData(0, 0, canvas.width, canvas.height).data;
    const colors = new Set();
    const stride = Math.max(4, Math.floor(data.length / 4096 / 4) * 4);
    for (let index = 0; index < data.length; index += stride) {
      colors.add(`${data[index]},${data[index + 1]},${data[index + 2]}`);
      if (colors.size >= 3) return true;
    }
    return false;
  }, selector);
}

async function assertNoViewportOverflow(page) {
  const layout = await page.evaluate(() => ({
    viewportWidth: window.innerWidth,
    viewportHeight: window.innerHeight,
    scrollWidth: document.documentElement.scrollWidth,
    scrollHeight: document.documentElement.scrollHeight,
    bodyScrollWidth: document.body.scrollWidth,
    bodyScrollHeight: document.body.scrollHeight,
  }));
  assert.ok(
    layout.scrollWidth <= layout.viewportWidth &&
      layout.bodyScrollWidth <= layout.viewportWidth,
    `horizontal overflow: ${JSON.stringify(layout)}`,
  );
  assert.ok(
    layout.scrollHeight <= layout.viewportHeight &&
      layout.bodyScrollHeight <= layout.viewportHeight,
    `vertical overflow: ${JSON.stringify(layout)}`,
  );
}

async function setAudioTime(page, seconds) {
  await page.locator("#audio").evaluate((audio, time) => {
    audio.currentTime = time;
  }, seconds);
  await expectAudioTime(page, seconds, 0.2);
}

async function expectAudioTime(page, expected, tolerance) {
  await page.waitForFunction(
    ({ target, allowed }) =>
      Math.abs(document.querySelector("#audio").currentTime - target) <= allowed,
    { target: expected, allowed: tolerance },
  );
}

function audioTime(page) {
  return page.locator("#audio").evaluate((audio) => audio.currentTime);
}

function audioSource(page) {
  return page.locator("#audio").evaluate((audio) => audio.currentSrc);
}
