import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright-core";
import { startPreviewServer } from "../preview-server.mjs";

const appDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const conv9Dir = resolve(appDir, "..");
const catalog = await testCatalog();
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

  browser = await chromium.launch({
    executablePath: chromeExecutable,
    headless: true,
    args: ["--no-sandbox", "--autoplay-policy=no-user-gesture-required"],
  });
  const page = await browser.newPage({ viewport: { width: 1280, height: 860 } });
  await page.addInitScript((injectedCatalog) => {
    window.__CONV9_TEST_REQUESTS__ = [];
    window.__CONV9_TEST_LATEST__ = 0;
    window.__CONV9_STATUS_HISTORY__ = [];
    document.addEventListener("DOMContentLoaded", () => {
      const status = document.querySelector("#renderStatus");
      const record = () => {
        window.__CONV9_STATUS_HISTORY__.push({
          status: status?.textContent,
          playDisabled: document.querySelector("#playButton")?.disabled,
          seekDisabled: document.querySelector("#seek")?.disabled,
        });
      };
      new MutationObserver(record).observe(status, {
        childList: true,
        characterData: true,
        subtree: true,
      });
      record();
    });
    window.__CONV9_TEST_BRIDGE__ = {
      loadBootstrap: async () => ({ catalog: injectedCatalog }),
      supersedeRender: (requestId) => {
        window.__CONV9_TEST_LATEST__ = requestId;
      },
      renderSelection: async (request) => {
        window.__CONV9_TEST_REQUESTS__.push(structuredClone(request));
        await new Promise((resolveDelay) => setTimeout(resolveDelay, 240));
        const response = await fetch(`/samples/prepared/${request.leftId}.wav`);
        if (!response.ok) throw new Error(`fixture WAV failed: ${response.status}`);
        const wav = new Uint8Array(await response.arrayBuffer());
        const outputDuration =
          request.algorithm === "full_convolution"
            ? request.parameters.full_a_duration_seconds +
              request.parameters.full_b_duration_seconds -
              1 / 48_000
            : 60;
        return {
          header: {
            requestId: request.requestId,
            leftId: request.leftId,
            rightId: request.rightId,
            algorithm: request.algorithm,
            windows: structuredClone(request.windows),
            hopSeconds:
              request.windows.clip_a_seconds == null
                ? null
                : request.algorithm === "chunk_crossfade"
                  ? Math.max(
                      request.windows.clip_a_seconds,
                      request.windows.clip_b_seconds,
                    )
                  : Math.min(
                      request.windows.clip_a_seconds,
                      request.windows.clip_b_seconds,
                    ) *
                    (1 - request.parameters.window_overlap_percent / 100),
            renderMilliseconds: 240,
            metrics: {
              frames: 2_880_000,
              duration_seconds: outputDuration,
              peak: 0.72,
              rms: 0.09,
              rms_dbfs: -20.9,
              dc_offset: 0,
              clipped_samples: 0,
              non_finite_samples: 0,
            },
          },
          wav,
        };
      },
    };
  }, catalog);
  page.on("pageerror", (error) => pageErrors.push(error.message));
  page.on("response", (response) => {
    if (response.status() >= 400) {
      failedResponses.push(`${response.status()} ${response.url()}`);
    }
  });

  await page.goto(`${baseUrl}/app/src/`, { waitUntil: "domcontentloaded" });
  await page.waitForFunction(
    () =>
      document.querySelector("#renderStatus")?.textContent === "rendering…" &&
      document.querySelector("#playButton")?.disabled,
  );
  await page.locator("#playButton").evaluate((button) => button.click());
  assert.equal(
    await page.locator("#audio").evaluate((audio) => audio.paused),
    true,
    "startup play is inert until rendering and analysis are complete",
  );
  await waitForReady(page);
  assert.equal(await page.locator("#playButton").isEnabled(), true);
  assert.equal(await page.locator("#seek").isEnabled(), true);
  const statusHistory = await page.evaluate(() => window.__CONV9_STATUS_HISTORY__);
  assert.ok(
    statusHistory.some(
      (entry) =>
        entry.status === "analyzing…" &&
        entry.playDisabled === true &&
        entry.seekDisabled === true,
    ),
    `transport was not locked during visualization analysis: ${JSON.stringify(statusHistory)}`,
  );

  assert.equal(await page.locator("#sourceASelect option").count(), 12, "clip A count");
  assert.equal(await page.locator("#sourceBSelect option").count(), 12, "clip B count");
  assert.equal(await page.locator("#algorithmButtons button").count(), 5, "algorithm count");
  assert.equal(await page.locator("#methodTools .window-control").count(), 2);
  assert.equal(await page.locator("#methodTools .tool-control").count(), 12);
  assert.equal(
    await page.getByLabel("A window exact value").getAttribute("min"),
    "0.1",
  );
  assert.equal(
    await page.getByLabel("A window exact value").getAttribute("max"),
    "30",
  );
  assert.equal(await page.getByLabel("A window exact value").inputValue(), "5.00");
  assert.equal(await page.getByLabel("B window exact value").inputValue(), "5.00");
  assert.equal(await page.getByLabel("overlap exact value").inputValue(), "35");
  const defaultSliderPosition = Number(
    await page.locator("input[type='range'][aria-label='A window']").inputValue(),
  );
  assert.ok(
    defaultSliderPosition >= 580 && defaultSliderPosition <= 640,
    `soft-log 5-second position is unexpected: ${defaultSliderPosition}`,
  );
  assert.equal(await page.locator("#audio").evaluate((audio) => audio.loop), true);
  await assertAudioReady(page, "/multiresolution/5.00x5.00");
  await assertCanvasHasVariation(page, "#waveform");
  await assertCanvasHasVariation(page, "#spectrogram");
  assert.equal(await page.locator("#spectrogram").getAttribute("data-fft-size"), "16384");
  assert.ok(
    Number(await page.locator("#spectrogram").getAttribute("data-analysis-columns")) > 720,
    "spectrogram time resolution exceeds the old 720-column cap",
  );
  await assertNoViewportOverflow(page);
  await assertControlTooltips(page);
  assert.match(
    await page.getByRole("button", { name: "multi", exact: true }).getAttribute("title"),
    /low, mid, and high bands/,
  );
  assert.match(
    await page.locator("input[type='range'][aria-label='A window']").getAttribute("title"),
    /seconds are extracted from clip A.*softened logarithmic curve/,
  );
  assert.equal(await page.locator("#errorPanel").isHidden(), true, "error panel hidden");
  if (process.env.CONV9_TEST_SCREENSHOT) {
    await page.screenshot({ path: process.env.CONV9_TEST_SCREENSHOT });
  }

  await setAudioTime(page, 24);
  await page.getByLabel("A window exact value").fill("1.37");
  await page.getByLabel("overlap exact value").fill("42");
  await waitForPath(page, "window_overlap_percent=42.00");
  await expectAudioTime(page, 24, 0.5);

  const thirdSource = await page.locator("#sourceBSelect option").nth(2).getAttribute("value");
  await page.locator("#sourceBSelect").selectOption(thirdSource);
  await waitForPath(page, `${thirdSource}/multiresolution/1.37x5.00`);

  await page.getByRole("button", { name: "chunks", exact: true }).click();
  await waitForPath(page, "/chunk_crossfade/5.00x5.00");
  assert.equal(await page.locator("#methodTools .window-control").count(), 2);
  assert.equal(await page.locator("#methodTools .tool-control").count(), 6);
  assert.equal(await page.getByLabel("overlap exact value").inputValue(), "25");
  assert.match(
    await page.getByLabel("overlap exact value").getAttribute("title"),
    /equal-power overlap/,
  );
  await assertControlTooltips(page);
  await page.getByLabel("overlap exact value").fill("40");
  await waitForPath(page, "chunk_crossfade_percent=40.00");
  await expectContains(page, "#windowReadout", "40%");

  await page.getByRole("button", { name: "full", exact: true }).click();
  await waitForPath(page, "/full_convolution/a0.00+60.00_b0.00+60.00");
  assert.equal(await page.locator("#methodTools .window-control").count(), 0);
  assert.equal(await page.locator("#methodTools .tool-control").count(), 4);
  assert.equal(await page.locator("#methodTools input").count(), 8);
  assert.equal(await page.getByLabel("A offset exact value").inputValue(), "0.0");
  assert.equal(await page.getByLabel("A duration exact value").inputValue(), "60.0");
  await expectContains(page, "#windowReadout", "out 120.00s");
  await assertControlTooltips(page);

  await page.getByLabel("A offset exact value").fill("10");
  await waitForPath(page, "/full_convolution/a10.00+50.00_b0.00+60.00");
  assert.equal(
    await page.getByLabel("A duration exact value").inputValue(),
    "50.0",
    "offset keeps the A segment inside its source",
  );
  await page.getByLabel("A offset exact value").fill("50");
  assert.equal(await page.getByLabel("A duration exact value").inputValue(), "10.0");
  await page.getByLabel("A duration exact value").fill("20");
  assert.equal(
    await page.getByLabel("A offset exact value").inputValue(),
    "40.0",
    "duration moves the A segment earlier when its end would exceed the source",
  );
  await page.getByLabel("A offset exact value").fill("10");
  await page.getByLabel("B duration exact value").fill("15");
  await waitForPath(page, "/full_convolution/a10.00+20.00_b0.00+15.00");
  await expectContains(page, "#windowReadout", "out 35.00s");

  await page.getByRole("button", { name: "Play" }).click();
  await page.waitForFunction(() => !document.querySelector("#audio").paused);
  const beforeSwitch = await audioTime(page);
  await page.getByRole("button", { name: "wola", exact: true }).click();
  await page.waitForFunction(
    () => document.querySelector("#renderStatus")?.textContent === "rendering…",
  );
  await page.getByRole("button", { name: "ir", exact: true }).click();
  await waitForPath(page, "/evolving_ir/5.00x5.00");
  assert.ok((await audioTime(page)) >= beforeSwitch - 0.5, "switch preserves position");
  assert.equal(await page.locator("#audio").evaluate((audio) => audio.paused), false);
  assert.match(await page.locator("#playButton").getAttribute("title"), /Pause playback/);
  const requests = await page.evaluate(() => window.__CONV9_TEST_REQUESTS__);
  assert.equal(requests.at(-1).algorithm, "evolving_ir", "latest rapid selection wins");

  await page.getByRole("button", { name: "Pause" }).click();
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
  await page.waitForTimeout(1_500);
  assert.equal(await audioSource(page), sourceBeforeResize, "resize does not reload audio");
  await expectAudioTime(page, timeBeforeResize, 0.2);
  await assertCanvasHasVariation(page, "#waveform");
  await assertCanvasHasVariation(page, "#spectrogram");
  await assertNoViewportOverflow(page);

  assert.deepEqual(pageErrors, [], `page errors: ${pageErrors.join("\n")}`);
  assert.deepEqual(failedResponses, [], `failed responses: ${failedResponses.join("\n")}`);
  console.log("conv9 browser functional test passed");
} finally {
  await browser?.close();
  if (server) {
    await new Promise((resolveClose, rejectClose) => {
      server.close((error) => (error ? rejectClose(error) : resolveClose()));
    });
  }
}

async function testCatalog() {
  const manifest = await readFile(resolve(conv9Dir, "sources.tsv"), "utf8");
  const [header, ...lines] = manifest.trim().split("\n").map((line) => line.split("\t"));
  const sources = lines.map((values) =>
    Object.fromEntries(header.map((name, index) => [name, numeric(name, values[index])]))
  );
  const windows = () => [
    {
      id: "clip_a_seconds",
      label: "A window",
      minimum: 0.1,
      maximum: 30,
      step: 0.01,
      default: 5,
      scale: "soft_log",
      description:
        "Sets how many seconds are extracted from clip A at each synchronized timeline position. Longer windows retain more context but create more temporal smear.",
    },
    {
      id: "clip_b_seconds",
      label: "B window",
      minimum: 0.1,
      maximum: 30,
      step: 0.01,
      default: 5,
      scale: "soft_log",
      description:
        "Sets how many seconds are extracted from clip B at each synchronized timeline position. Longer windows retain more context but create more temporal smear.",
    },
  ];
  const crossfadeShape = {
    id: "taper",
    label: "crossfade",
    minimum: 0.05,
    maximum: 1,
    step: 0.01,
    default: 0.5,
    unit: "",
    description:
      "Shapes both the Tukey analysis window and normalized raised-cosine synthesis crossfade between overlapping results.",
  };
  const analysisTaper = {
    ...crossfadeShape,
    label: "edge taper",
    description:
      "Sets the Tukey taper on each independent input chunk; chunk blending is controlled by the separate overlap percentage.",
  };
  const parameter = (id, label, minimum, maximum, step, defaultValue, unit = "") => ({
    id,
    label,
    minimum,
    maximum,
    step,
    default: defaultValue,
    unit,
    description: {
      multires_low_scale:
        "Multiplies both windows for the low-frequency band. Larger values stabilize bass but add smear.",
      multires_high_scale:
        "Multiplies both windows for the high-frequency band. Smaller values sharpen transients and timing.",
      multires_low_mix:
        "Scales the low-frequency convolution band before all bands are recombined and conditioned.",
      multires_high_mix:
        "Scales the high-frequency convolution band before all bands are recombined and conditioned.",
      multires_low_split_hz:
        "Sets the smooth low-to-mid frequency split center used by the multiresolution method.",
      multires_high_split_hz:
        "Sets the smooth mid-to-high frequency split center used by the multiresolution method.",
      multires_transition_width:
        "Sets the normalized raised-cosine crossover width; wider values blend neighboring bands more broadly.",
      window_overlap_percent:
        "Sets how much of the shorter analysis window overlaps its next position and controls render density.",
      window_b_offset_seconds:
        "Offsets clip B's scan relative to clip A, with reflected source boundaries and no dry audio.",
      evolving_a_mix:
        "Blends the cropped carriers: zero keeps B, one keeps A, and one half weights both equally.",
      evolving_mix_motion:
        "Moves the A/B carrier balance over the output timeline around the selected midpoint value.",
      evolving_crop_position:
        "Chooses the onset, center, or tail region used for each cropped evolving convolution carrier.",
      chunk_crossfade_percent:
        "Sets equal-power overlap as a percentage of the shorter chunk. Higher values make longer, smoother transitions.",
      chunk_crop_position:
        "Chooses the onset, center, or tail region cropped from every complete local chunk convolution.",
      full_a_offset_seconds:
        "Sets the beginning of clip A's selected segment and automatically keeps its duration within the source.",
      full_a_duration_seconds:
        "Sets how much of clip A is selected for full convolution and defaults to the complete source.",
      full_b_offset_seconds:
        "Sets the beginning of clip B's selected segment and automatically keeps its duration within the source.",
      full_b_duration_seconds:
        "Sets how much of clip B is selected for full convolution and defaults to the complete source.",
    }[id],
  });
  return {
    schema_version: 4,
    mode: "on_demand",
    sample_rate: 48_000,
    channels: 1,
    input_seconds: 60,
    sources,
    algorithms: [
      {
        id: "multiresolution",
        title: "Multiresolution convolution",
        description:
          "Splits local convolution into complementary low, mid, and high bands, uses different window scales, then recombines them.",
        rank: 1,
        windows: windows(),
        parameters: [
          crossfadeShape,
          parameter("window_overlap_percent", "overlap", 5, 80, 1, 35, "%"),
          parameter("window_b_offset_seconds", "B offset", -30, 30, 0.1, 0, "s"),
          parameter("multires_low_scale", "low scale", 1, 3, 0.05, 1.6, "×"),
          parameter("multires_high_scale", "high scale", 0.15, 1, 0.01, 0.6, "×"),
          parameter("multires_low_mix", "low gain", 0, 2, 0.01, 0.9, "×"),
          parameter("multires_high_mix", "high gain", 0, 2, 0.01, 0.62, "×"),
          parameter("multires_low_split_hz", "low split", 80, 800, 5, 230, "hz"),
          parameter("multires_high_split_hz", "high split", 800, 8000, 25, 2100, "hz"),
          parameter("multires_transition_width", "split width", 0.05, 0.75, 0.01, 0.3),
        ],
      },
      {
        id: "sliding_wola",
        title: "Sliding WOLA convolution",
        description:
          "Convolves synchronized windows and merges them with normalized raised-cosine overlap-add for a neutral local baseline.",
        rank: 2,
        windows: windows(),
        parameters: [
          crossfadeShape,
          parameter("window_overlap_percent", "overlap", 5, 80, 1, 35, "%"),
          parameter("window_b_offset_seconds", "B offset", -30, 30, 0.1, 0, "s"),
        ],
      },
      {
        id: "evolving_ir",
        title: "Dual evolving impulse response",
        description:
          "Crops every local convolution into A-sized and B-sized carriers, blends them, and overlap-adds the result.",
        rank: 3,
        windows: windows(),
        parameters: [
          crossfadeShape,
          parameter("window_overlap_percent", "overlap", 5, 80, 1, 35, "%"),
          parameter("window_b_offset_seconds", "B offset", -30, 30, 0.1, 0, "s"),
          parameter("evolving_a_mix", "A carrier", 0, 1, 0.01, 0.5),
          parameter("evolving_mix_motion", "carrier motion", -1, 1, 0.01, 0),
          parameter("evolving_crop_position", "crop position", 0, 1, 0.01, 0.5),
        ],
      },
      {
        id: "chunk_crossfade",
        title: "Independent chunks + crossfade",
        description:
          "Convolves synchronized chunks and joins adjacent results with an equal-power crossfade of configurable length.",
        rank: 4,
        windows: windows(),
        parameters: [
          analysisTaper,
          parameter("chunk_crossfade_percent", "overlap", 5, 75, 1, 25, "%"),
          parameter("window_b_offset_seconds", "B offset", -30, 30, 0.1, 0, "s"),
          parameter("chunk_crop_position", "crop position", 0, 1, 0.01, 0.5),
        ],
      },
      {
        id: "full_convolution",
        title: "Full linear convolution",
        description:
          "Selects one segment from each clip, convolves them as the smear reference, and retains the complete linear result.",
        rank: 5,
        windows: [],
        parameters: [
          parameter("full_a_offset_seconds", "A offset", 0, 59.9, 0.1, 0, "s"),
          parameter("full_a_duration_seconds", "A duration", 0.1, 60, 0.1, 60, "s"),
          parameter("full_b_offset_seconds", "B offset", 0, 59.9, 0.1, 0, "s"),
          parameter("full_b_duration_seconds", "B duration", 0.1, 60, 0.1, 60, "s"),
        ],
      },
    ],
  };
}

function numeric(name, value) {
  return ["seconds", "trim_start"].includes(name) ? Number(value) : value;
}

async function waitForReady(page) {
  await page.waitForFunction(() => {
    const audio = document.querySelector("#audio");
    const waveform = document.querySelector("#waveform");
    const spectrum = document.querySelector("#spectrogram");
    return (
      audio?.readyState >= HTMLMediaElement.HAVE_METADATA &&
      waveform?.getAttribute("aria-busy") === "false" &&
      spectrum?.getAttribute("aria-busy") === "false" &&
      document.querySelector("#renderStatus")?.textContent.startsWith("rendered ")
    );
  }, { timeout: 120_000 });
}

async function waitForPath(page, pathPart) {
  await page.waitForFunction(
    (part) =>
      document.querySelector("#audio").dataset.path.includes(part) &&
      document.querySelector("#renderStatus")?.textContent.startsWith("rendered "),
    pathPart,
    { timeout: 120_000 },
  );
}

async function expectContains(page, selector, expected) {
  await page.waitForFunction(
    ({ target, text }) => document.querySelector(target)?.textContent.includes(text),
    { target: selector, text: expected },
  );
}

async function assertAudioReady(page, pathPart) {
  await page.waitForFunction(
    (part) => {
      const audio = document.querySelector("#audio");
      return (
        audio.readyState >= HTMLMediaElement.HAVE_METADATA &&
        Math.abs(audio.duration - 60) < 0.01 &&
        audio.currentSrc.startsWith("blob:") &&
        audio.dataset.path.includes(part)
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

async function assertControlTooltips(page) {
  const missing = await page.locator("button, select, input, canvas, a[href]").evaluateAll(
    (controls) =>
      controls
        .filter(
          (control) =>
            !control.title ||
            control.title.trim().length < 40 ||
            control.title.includes("undefined"),
        )
        .map(
          (control) =>
            `${control.tagName.toLowerCase()}#${control.id || control.getAttribute("aria-label") || control.textContent}`,
        ),
  );
  assert.deepEqual(missing, [], `controls without explanatory hover tooltips: ${missing}`);
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
