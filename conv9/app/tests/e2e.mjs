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
            : 61;
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
              frames: 2_928_000,
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

  assert.equal(await page.locator("#sourceASelect option").count(), 48, "clip A count");
  assert.equal(await page.locator("#sourceBSelect option").count(), 48, "clip B count");
  assert.equal(
    await page.locator(".clip-selectors label, .source-meta, .convolution-mark").count(),
    0,
    "selector row contains only the two source controls",
  );
  assert.equal(
    await page.locator("#methodToolTitle, .method-panel > header").count(),
    0,
    "method controls have no redundant heading",
  );
  assert.equal(
    await page.locator("h1, .method-field, .field-label").count(),
    0,
    "method buttons are left-aligned without app or method captions",
  );
  assert.equal(await page.locator("#algorithmButtons button").count(), 6, "algorithm count");
  const uiScale = await page.evaluate(() => {
    const style = (selector) => getComputedStyle(document.querySelector(selector));
    return {
      buttonFont: style("#algorithmButtons button").fontSize,
      selectFont: style("#sourceASelect").fontSize,
      numberFont: style("#methodTools input[type='number']").fontSize,
      statusFont: style("#renderStatus").fontSize,
      readoutFont: style("#windowReadout").fontSize,
      metricFont: style("#metrics dd").fontSize,
      timeFont: style("#currentTime").fontSize,
      speedValueFont: style("#playbackSpeedValue").fontSize,
      toolLabelFont: style(".tool-control > span").fontSize,
      plotLabelFont: style(".visual-card header").fontSize,
      buttonHeight: style("#algorithmButtons button").height,
      selectHeight: style("#sourceASelect").height,
      sliderHeight: style("#methodTools input[type='range']").height,
    };
  });
  assert.deepEqual(uiScale, {
    buttonFont: "16px",
    selectFont: "16px",
    numberFont: "16px",
    statusFont: "16px",
    readoutFont: "16px",
    metricFont: "16px",
    timeFont: "16px",
    speedValueFont: "16px",
    toolLabelFont: "16px",
    plotLabelFont: "16px",
    buttonHeight: "36px",
    selectHeight: "38px",
    sliderHeight: "18px",
  });
  await assertNoUndersizedText(page);
  await assertToolLabelsFit(page);
  assert.equal(
    await page.locator("#renderStatus").evaluate((status) =>
      getComputedStyle(status).position
    ),
    "absolute",
  );
  const methodBoundsBeforeStatusChange = await page.locator("#algorithmButtons").boundingBox();
  await page.locator("#renderStatus").evaluate((status) => {
    status.dataset.previousText = status.textContent;
    status.textContent = "rendered 123456789 ms";
  });
  const methodBoundsAfterStatusChange = await page.locator("#algorithmButtons").boundingBox();
  assert.deepEqual(
    methodBoundsAfterStatusChange,
    methodBoundsBeforeStatusChange,
    "floating render status must not move or resize method controls",
  );
  await page.locator("#renderStatus").evaluate((status) => {
    status.textContent = status.dataset.previousText;
    delete status.dataset.previousText;
  });
  assert.equal(await page.locator("#methodTools .window-control").count(), 2);
  assert.equal(await page.locator("#methodTools .tool-control").count(), 4);
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
  assert.equal(await page.getByLabel("input taper exact value").inputValue(), "0.50");
  assert.equal(await page.getByLabel("overlap exact value").inputValue(), "75");
  const defaultSliderPosition = Number(
    await page.locator("input[type='range'][aria-label='A window']").inputValue(),
  );
  assert.ok(
    defaultSliderPosition >= 580 && defaultSliderPosition <= 640,
    `soft-log 5-second position is unexpected: ${defaultSliderPosition}`,
  );
  assert.equal(await page.locator("#audio").evaluate((audio) => audio.loop), true);
  await assertAudioReady(page, "/windowed_convolution/5.00x5.00");
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
    await page.getByRole("button", { name: "windowed", exact: true }).getAttribute("title"),
    /one ordinary linear FFT convolution/,
  );
  assert.match(
    await page.locator("input[type='range'][aria-label='A window']").getAttribute("title"),
    /seconds are extracted from clip A.*softened logarithmic curve/,
  );
  assert.match(
    await page.getByLabel("input taper exact value").getAttribute("title"),
    /both extracted input windows.*synthesis crossfading remains separate/,
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
  await waitForPath(page, `${thirdSource}/windowed_convolution/1.37x5.00`);

  await page.getByRole("button", { name: "chunks", exact: true }).click();
  await waitForPath(page, "/chunk_crossfade/5.00x5.00");
  assert.equal(await page.locator("#methodTools .window-control").count(), 2);
  assert.equal(await page.locator("#methodTools .tool-control").count(), 5);
  assert.equal(await page.getByLabel("overlap exact value").inputValue(), "50");
  assert.match(
    await page.getByLabel("overlap exact value").getAttribute("title"),
    /power-normalized overlap/,
  );
  await assertControlTooltips(page);
  await page.getByLabel("overlap exact value").fill("40");
  await waitForPath(page, "chunk_crossfade_percent=40.00");
  await expectContains(page, "#windowReadout", "40%");

  await page.getByRole("button", { name: "full", exact: true }).click();
  await waitForPath(page, "/full_convolution/a0.00+61.00_b0.00+61.00");
  assert.equal(await page.locator("#methodTools .window-control").count(), 0);
  assert.equal(await page.locator("#methodTools .tool-control").count(), 4);
  assert.equal(await page.locator("#methodTools input").count(), 8);
  assert.equal(await page.getByLabel("A offset exact value").inputValue(), "0.0");
  assert.equal(await page.getByLabel("A duration exact value").inputValue(), "61.0");
  await expectContains(page, "#windowReadout", "out 122.00s");
  await assertControlTooltips(page);

  await page.getByLabel("A offset exact value").fill("10");
  await waitForPath(page, "/full_convolution/a10.00+51.00_b0.00+61.00");
  assert.equal(
    await page.getByLabel("A duration exact value").inputValue(),
    "51.0",
    "offset keeps the A segment inside its source",
  );
  await page.getByLabel("A offset exact value").fill("50");
  assert.equal(await page.getByLabel("A duration exact value").inputValue(), "11.0");
  await page.getByLabel("A duration exact value").fill("20");
  assert.equal(
    await page.getByLabel("A offset exact value").inputValue(),
    "41.0",
    "duration moves the A segment earlier when its end would exceed the source",
  );
  await page.getByLabel("A offset exact value").fill("10");
  await page.getByLabel("B duration exact value").fill("15");
  await waitForPath(page, "/full_convolution/a10.00+20.00_b0.00+15.00");
  await expectContains(page, "#windowReadout", "out 35.00s");

  await page.getByRole("button", { name: "dry a", exact: true }).click();
  await waitForPath(page, "/dry_a/source");
  assert.equal(await page.locator("#methodTools .window-control").count(), 0);
  assert.equal(await page.locator("#methodTools .tool-control").count(), 0);
  await expectContains(page, "#methodTools", "no configurable parameters");
  await expectContains(page, "#windowReadout", "source a / out 61.00s");

  await page.getByRole("button", { name: "dry b", exact: true }).click();
  await waitForPath(page, "/dry_b/source");
  await expectContains(page, "#windowReadout", "source b / out 61.00s");

  await page.getByRole("button", { name: "Play" }).click();
  await page.waitForFunction(() => !document.querySelector("#audio").paused);
  const beforeSwitch = await audioTime(page);
  await page.getByRole("button", { name: "chunks", exact: true }).click();
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
  assert.equal(await page.locator("#audio").evaluate((audio) => audio.playbackRate), 1);
  assert.deepEqual(
    await page.locator("#playbackSpeed").evaluate((input) => ({
      minimum: input.min,
      maximum: input.max,
      value: input.value,
      normalizedPosition: (Number(input.value) - Number(input.min)) /
        (Number(input.max) - Number(input.min)),
    })),
    { minimum: "-1", maximum: "1", value: "0", normalizedPosition: 0.5 },
  );
  await page.locator("#playbackSpeed").evaluate((input) => {
    input.value = "0.5";
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  assert.ok(
    Math.abs(
      (await page.locator("#audio").evaluate((audio) => audio.playbackRate)) - Math.SQRT2,
    ) < 1e-6,
  );
  assert.equal(await page.locator("#playbackSpeedValue").textContent(), "1.41×");
  assert.equal(
    await page.locator("#playbackSpeed").getAttribute("aria-valuetext"),
    "1.41 times",
  );

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
  await expectAudioTime(page, 45.75, 0.5);

  const sourceBeforeResize = await audioSource(page);
  const timeBeforeResize = await audioTime(page);
  await page.setViewportSize({ width: 900, height: 640 });
  await page.waitForTimeout(1_500);
  assert.equal(await audioSource(page), sourceBeforeResize, "resize does not reload audio");
  await expectAudioTime(page, timeBeforeResize, 0.2);
  await assertCanvasHasVariation(page, "#waveform");
  await assertCanvasHasVariation(page, "#spectrogram");
  await assertNoViewportOverflow(page);
  await assertNoUndersizedText(page);
  await assertToolLabelsFit(page);
  if (process.env.CONV9_TEST_SCREENSHOT) {
    await page.screenshot({ path: `${process.env.CONV9_TEST_SCREENSHOT}.compact.png` });
  }

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
  const parameter = (id, label, minimum, maximum, step, defaultValue, unit = "") => ({
    id,
    label,
    minimum,
    maximum,
    step,
    default: defaultValue,
    unit,
    description: {
      window_overlap_percent:
        "Sets how much of the shorter analysis window overlaps its next position and controls render density.",
      input_taper:
        "Sets the Tukey taper applied to both extracted input windows before each convolution; synthesis crossfading remains separate.",
      evolving_a_mix:
        "Blends the cropped carriers: zero keeps B, one keeps A, and one half weights both equally.",
      evolving_mix_motion:
        "Moves the A/B carrier balance over the output timeline around the selected midpoint value.",
      evolving_crop_position:
        "Chooses the onset, center, or tail region used for each cropped evolving convolution carrier.",
      chunk_crossfade_percent:
        "Sets power-normalized overlap as a percentage of the shorter chunk. The 50% default keeps transitions continuous; lower values expose seams.",
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
    schema_version: 7,
    mode: "on_demand",
    sample_rate: 48_000,
    channels: 1,
    input_seconds: 61,
    sources,
    algorithms: [
      {
        id: "windowed_convolution",
        title: "Windowed convolution",
        description:
          "Extracts synchronized windows, performs one ordinary linear FFT convolution for each pair, and coherence-normalizes their root-Hann crossfades.",
        rank: 1,
        windows: windows(),
        parameters: [
          parameter("input_taper", "input taper", 0.05, 1, 0.01, 0.5),
          parameter("window_overlap_percent", "overlap", 5, 80, 1, 75, "%"),
        ],
      },
      {
        id: "evolving_ir",
        title: "Dual evolving impulse response",
        description:
          "Crops every local convolution into A-sized and B-sized carriers, blends them, and overlap-adds the result.",
        rank: 2,
        windows: windows(),
        parameters: [
          parameter("input_taper", "input taper", 0.05, 1, 0.01, 0.5),
          parameter("window_overlap_percent", "overlap", 5, 80, 1, 75, "%"),
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
        rank: 3,
        windows: windows(),
        parameters: [
          parameter("input_taper", "input taper", 0.05, 1, 0.01, 0.5),
          parameter("chunk_crossfade_percent", "overlap", 5, 75, 1, 50, "%"),
          parameter("chunk_crop_position", "crop position", 0, 1, 0.01, 0.5),
        ],
      },
      {
        id: "full_convolution",
        title: "Full linear convolution",
        description:
          "Selects one segment from each clip, convolves them as the smear reference, and retains the complete linear result.",
        rank: 4,
        windows: [],
        parameters: [
          parameter("full_a_offset_seconds", "A offset", 0, 60.9, 0.1, 0, "s"),
          parameter("full_a_duration_seconds", "A duration", 0.1, 61, 0.1, 61, "s"),
          parameter("full_b_offset_seconds", "B offset", 0, 60.9, 0.1, 0, "s"),
          parameter("full_b_duration_seconds", "B duration", 0.1, 61, 0.1, 61, "s"),
        ],
      },
      {
        id: "dry_a",
        title: "Dry source A",
        description:
          "Plays the complete conditioned clip A without convolution, output saturation, or a second level-normalization pass.",
        rank: 5,
        windows: [],
        parameters: [],
      },
      {
        id: "dry_b",
        title: "Dry source B",
        description:
          "Plays the complete conditioned clip B without convolution, output saturation, or a second level-normalization pass.",
        rank: 6,
        windows: [],
        parameters: [],
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
        Math.abs(audio.duration - 61) < 0.01 &&
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

async function assertNoUndersizedText(page) {
  const undersized = await page.locator("body *").evaluateAll((elements) =>
    elements
      .filter((element) => {
        const style = getComputedStyle(element);
        if (
          style.display === "none" ||
          style.visibility === "hidden" ||
          element.getClientRects().length === 0
        ) {
          return false;
        }
        const hasOwnText = [...element.childNodes].some(
          (node) => node.nodeType === Node.TEXT_NODE && node.textContent.trim(),
        );
        const isTextControl = element.matches("button, select, input, output");
        return (hasOwnText || isTextControl) && parseFloat(style.fontSize) < 16;
      })
      .map(
        (element) =>
          `${element.tagName.toLowerCase()}#${element.id || element.className || element.getAttribute("aria-label")}`,
      ),
  );
  assert.deepEqual(undersized, [], `rendered text below 16px: ${undersized.join(", ")}`);
}

async function assertToolLabelsFit(page) {
  const invalid = await page.locator(".tool-control").evaluateAll((controls) =>
    controls
      .filter((control) => {
        const label = control.querySelector(":scope > span");
        const inputs = control.querySelector(".tool-inputs");
        if (!label || !inputs) return true;
        const labelBounds = label.getBoundingClientRect();
        const inputBounds = inputs.getBoundingClientRect();
        return (
          label.scrollWidth > label.clientWidth + 1 ||
          label.scrollHeight > label.clientHeight + 1 ||
          labelBounds.bottom > inputBounds.top
        );
      })
      .map((control) => control.querySelector(":scope > span")?.textContent.trim()),
  );
  assert.deepEqual(invalid, [], `clipped or overlapping tool labels: ${invalid.join(", ")}`);
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
