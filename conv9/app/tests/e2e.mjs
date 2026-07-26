import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright-core";
import { startPreviewServer } from "../preview-server.mjs";

const appDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const conv9Dir = resolve(appDir, "..");
const windowTitle = "Convolution Playground";
const catalog = await testCatalog();
const tauriConfig = JSON.parse(
  await readFile(resolve(appDir, "src-tauri/tauri.conf.json"), "utf8"),
);
const iconSvg = await readFile(resolve(appDir, "src-tauri/icons/icon.svg"), "utf8");
const chromeExecutable =
  process.env.CHROME_BIN ||
  ["/usr/bin/google-chrome-stable", "/usr/bin/chromium", "/usr/bin/google-chrome"].find(
    (candidate) => existsSync(candidate),
  );

assert.ok(chromeExecutable, "Set CHROME_BIN to a Chrome or Chromium executable");
assert.equal(tauriConfig.productName, "Convolution Playground");
assert.equal(tauriConfig.app.windows[0].title, windowTitle);
assert.deepEqual(tauriConfig.bundle.icon, ["icons/icon.png"]);
assert.match(iconSvg, /<title id="title">Convolution Playground<\/title>/);
assert.ok(existsSync(resolve(appDir, "src-tauri/icons/icon.png")), "raster app icon is missing");

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
    window.__CONV9_TEST_PREVIEW_REQUESTS__ = [];
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
      loadBootstrap: async () => ({ catalog: injectedCatalog, renderEpoch: 7 }),
      supersedeRender: (_renderEpoch, requestId) => {
        window.__CONV9_TEST_LATEST__ = requestId;
      },
      loadSourcePreview: async (id, bins) => {
        window.__CONV9_TEST_PREVIEW_REQUESTS__.push(id);
        const response = await fetch(`/samples/prepared/${id}.wav`);
        if (!response.ok) throw new Error(`preview WAV failed: ${response.status}`);
        const bytes = await response.arrayBuffer();
        const view = new DataView(bytes);
        let dataOffset = 44;
        let dataLength = bytes.byteLength - dataOffset;
        for (let offset = 12; offset + 8 <= bytes.byteLength;) {
          const chunk = String.fromCharCode(
            view.getUint8(offset),
            view.getUint8(offset + 1),
            view.getUint8(offset + 2),
            view.getUint8(offset + 3),
          );
          const length = view.getUint32(offset + 4, true);
          if (chunk === "data") {
            dataOffset = offset + 8;
            dataLength = length;
            break;
          }
          offset += 8 + length + (length & 1);
        }
        const frameCount = Math.floor(dataLength / 2);
        const peaks = [];
        let sumSquares = 0;
        let peak = 0;
        let crossings = 0;
        let previous = view.getInt16(dataOffset, true);
        for (let index = 0; index < frameCount; index += 1) {
          const sample = view.getInt16(dataOffset + index * 2, true) / 32768;
          sumSquares += sample * sample;
          peak = Math.max(peak, Math.abs(sample));
          if ((sample < 0) !== (previous < 0)) crossings += 1;
          previous = sample;
        }
        for (let bin = 0; bin < bins; bin += 1) {
          const start = Math.floor((bin * frameCount) / bins);
          const end = Math.max(start + 1, Math.floor(((bin + 1) * frameCount) / bins));
          let minimum = 1;
          let maximum = -1;
          for (let index = start; index < end; index += 1) {
            const sample = view.getInt16(dataOffset + index * 2, true) / 32768;
            minimum = Math.min(minimum, sample);
            maximum = Math.max(maximum, sample);
          }
          peaks.push([minimum, maximum]);
        }
        const rms = Math.sqrt(sumSquares / frameCount);
        return {
          id,
          peaks,
          peak,
          rmsDbfs: 20 * Math.log10(Math.max(rms, 1e-12)),
          zeroCrossingRate: crossings / Math.max(1, frameCount - 1),
        };
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
            renderEpoch: request.renderEpoch,
            requestId: request.requestId,
            leftId: request.leftId,
            rightId: request.rightId,
            algorithm: request.algorithm,
            windows: structuredClone(request.windows),
            parameters: structuredClone(request.parameters),
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
  assert.equal(await page.title(), windowTitle);
  await page
    .waitForFunction(
      () =>
        document.querySelector("#renderStatus")?.textContent === "rendering…" &&
        document.querySelector("#playButton")?.disabled,
    )
    .catch(async (error) => {
      const errorText = await page.locator("#errorPanel").textContent().catch(() => "");
      throw new Error(
        `startup did not render: ${[...pageErrors, errorText].filter(Boolean).join("\n")}`,
        { cause: error },
      );
    });
  if (process.env.CONV9_TEST_SCREENSHOT) {
    await page.screenshot({ path: `${process.env.CONV9_TEST_SCREENSHOT}.loading.png` });
  }
  await page.locator("#playButton").evaluate((button) => button.click());
  assert.equal(
    await page.evaluate(() => transportSnapshot().paused),
    true,
    "startup play is inert until rendering and analysis are complete",
  );
  if (process.env.CONV9_TEST_SCREENSHOT) {
    await page.waitForFunction(
      () => document.querySelector("#renderStatus")?.textContent === "analyzing…",
    );
    await page.screenshot({ path: `${process.env.CONV9_TEST_SCREENSHOT}.analyzing.png` });
  }
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

  assert.equal(await page.locator("#sourceASelect option").count(), 96, "clip A count");
  assert.equal(await page.locator("#sourceBSelect option").count(), 96, "clip B count");
  assert.equal(
    await page.evaluate(() => window.__CONV9_TEST_PREVIEW_REQUESTS__.length),
    0,
    "source waveforms are not loaded at startup",
  );
  const swapControl = await page.evaluate(() => {
    const a = document.querySelector("#sourceABrowser .source-browser-trigger");
    const swap = document.querySelector("#swapSources");
    const b = document.querySelector("#sourceBBrowser .source-browser-trigger");
    const bounds = (element) => {
      const box = element.getBoundingClientRect();
      return { left: box.left, right: box.right, height: box.height };
    };
    return {
      aValue: document.querySelector("#sourceASelect").value,
      bValue: document.querySelector("#sourceBSelect").value,
      aBounds: bounds(a),
      swapBounds: bounds(swap),
      bBounds: bounds(b),
      text: swap.textContent.trim(),
      iconCount: swap.querySelectorAll(":scope > svg").length,
      label: swap.getAttribute("aria-label"),
    };
  });
  assert.equal(swapControl.text, "", "source swap is icon-only");
  assert.equal(swapControl.iconCount, 1, "source swap contains one SVG icon");
  assert.equal(swapControl.label, "Swap clips A and B");
  assert.ok(
    swapControl.aBounds.right <= swapControl.swapBounds.left &&
      swapControl.swapBounds.right <= swapControl.bBounds.left &&
      swapControl.swapBounds.height === swapControl.aBounds.height,
    `source swap must sit between equal-height selectors: ${JSON.stringify(swapControl)}`,
  );
  await page.locator("#swapSources").click();
  await waitForPath(
    page,
    `${swapControl.bValue}__${swapControl.aValue}/windowed_convolution/`,
  );
  assert.deepEqual(
    await page.evaluate(() => ({
      a: document.querySelector("#sourceASelect").value,
      b: document.querySelector("#sourceBSelect").value,
    })),
    { a: swapControl.bValue, b: swapControl.aValue },
    "source swap reverses both dropdown values and the rendered source roles",
  );
  await page.locator("#sourceABrowser .source-browser-trigger").click();
  const sourceDialog = page.locator("#source-a-dialog");
  await sourceDialog.waitFor({ state: "visible" });
  const initialDialogBounds = await sourceDialog.boundingBox();
  const initialPreviewBounds = await sourceDialog
    .locator(".source-preview-waveform")
    .boundingBox();
  assert.ok(
    initialDialogBounds?.width >= 940 && initialDialogBounds?.height >= 580,
    `source browser should use the available navigation space: ${JSON.stringify(initialDialogBounds)}`,
  );
  assert.equal(
    await sourceDialog.locator(".source-preview-stats > div").count(),
    4,
    "preview reserves all four summary-stat cells while loading",
  );
  assert.equal(await sourceDialog.getAttribute("role"), "dialog");
  assert.equal(
    await sourceDialog.locator(".source-list").getAttribute("role"),
    "listbox",
    "source results use listbox semantics",
  );
  const selectedOptionBounds = await sourceDialog
    .locator("[role='option'][aria-selected='true']")
    .boundingBox();
  const sourceListBounds = await sourceDialog.locator(".source-list").boundingBox();
  assert.ok(
    selectedOptionBounds &&
      sourceListBounds &&
      selectedOptionBounds.y >= sourceListBounds.y &&
      selectedOptionBounds.y + selectedOptionBounds.height <=
        sourceListBounds.y + sourceListBounds.height,
    `opening the browser should reveal the selected source: ${JSON.stringify({
      selectedOptionBounds,
      sourceListBounds,
    })}`,
  );
  await page.waitForFunction(
    () =>
      document.querySelector("#source-a-dialog .source-preview-waveform")
        ?.getAttribute("aria-busy") === "false" &&
      window.__CONV9_TEST_PREVIEW_REQUESTS__.length === 1,
  );
  const loadedPreviewBounds = await sourceDialog
    .locator(".source-preview-waveform")
    .boundingBox();
  assert.ok(
    initialPreviewBounds &&
      loadedPreviewBounds &&
      Math.abs(initialPreviewBounds.height - loadedPreviewBounds.height) < 1,
    `preview loading must not resize the waveform: ${JSON.stringify({
      initialPreviewBounds,
      loadedPreviewBounds,
    })}`,
  );
  await assertCanvasMatchesLayout(page, "#source-a-dialog .source-preview-waveform");
  const firstPreviewPixels = await canvasFingerprint(
    page,
    "#source-a-dialog .source-preview-waveform",
  );
  await sourceDialog.locator(".source-search").fill("helicopter");
  await page.waitForFunction(
    () =>
      document.querySelectorAll("#source-a-dialog [role='option']").length === 1 &&
      document.querySelector("#source-a-dialog .source-match-count")?.textContent === "1 / 96",
  );
  await page.waitForFunction(
    () =>
      document.querySelector("#source-a-dialog .source-preview-waveform")
        ?.getAttribute("aria-busy") === "false" &&
      window.__CONV9_TEST_PREVIEW_REQUESTS__.includes("helicopter_takeoff"),
  );
  const helicopterPreviewPixels = await canvasFingerprint(
    page,
    "#source-a-dialog .source-preview-waveform",
  );
  assert.notEqual(
    helicopterPreviewPixels,
    firstPreviewPixels,
    "highlighting a different source produces a different lazy waveform",
  );
  await sourceDialog.locator(".source-search").press("ArrowDown");
  assert.equal(
    await sourceDialog.locator(".source-list").getAttribute("aria-activedescendant"),
    "source-a-option-helicopter_takeoff",
  );
  await sourceDialog.locator(".source-list").press("Enter");
  await waitForPath(
    page,
    `helicopter_takeoff__${swapControl.aValue}/windowed_convolution/`,
  );
  assert.equal(
    await page.locator("#sourceASelect").inputValue(),
    "helicopter_takeoff",
    "keyboard selection updates the hidden state mirror and render request",
  );
  await page.locator("#sourceABrowser .source-browser-trigger").click();
  await sourceDialog.locator(".source-search").fill("");
  await sourceDialog.locator("[data-group='music']").click();
  const musicalMatches = await sourceDialog.locator("[role='option']").count();
  assert.ok(
    musicalMatches > 10 && musicalMatches < 96,
    `music category should meaningfully filter the catalog: ${musicalMatches}`,
  );
  await assertNoViewportOverflow(page);
  await assertNoUndersizedText(page);
  if (process.env.CONV9_TEST_SCREENSHOT) {
    await page.waitForFunction(
      () =>
        document.querySelector("#source-a-dialog .source-preview-waveform")
          ?.getAttribute("aria-busy") === "false",
    );
    await page.screenshot({ path: `${process.env.CONV9_TEST_SCREENSHOT}.browser.png` });
  }
  await sourceDialog.locator(".source-search").press("Escape");
  assert.equal(await sourceDialog.isHidden(), true, "Escape closes the source browser");
  assert.equal(
    await page.locator("#sourceABrowser .source-browser-trigger").evaluate(
      (trigger) => document.activeElement === trigger,
    ),
    true,
    "Escape restores focus to the source trigger",
  );
  const previewRequestsBeforeCacheCheck = await page.evaluate(
    () => window.__CONV9_TEST_PREVIEW_REQUESTS__.length,
  );
  await page.locator("#sourceABrowser .source-browser-trigger").click();
  await page.waitForTimeout(150);
  assert.equal(
    await page.evaluate(() => window.__CONV9_TEST_PREVIEW_REQUESTS__.length),
    previewRequestsBeforeCacheCheck,
    "reopening a selected source uses its cached waveform",
  );
  await sourceDialog.locator(".source-search").press("Escape");
  const selectedABeforeBrowsingB = await page.locator("#sourceASelect").inputValue();
  await page.locator("#sourceBBrowser .source-browser-trigger").click();
  await page.locator("#source-b-dialog").waitFor({ state: "visible" });
  assert.equal(
    await page.locator("#sourceASelect").inputValue(),
    selectedABeforeBrowsingB,
    "clip B browser has independent selection state",
  );
  await page.locator("#source-b-dialog .source-search").press("Escape");
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
  assert.equal(
    await page.locator("#renderTitle, #windowReadout, .visual-card > header, .now-playing").count(),
    0,
    "repeated render details and visualization headings are absent",
  );
  const waveformBounds = await page.locator("#waveform").boundingBox();
  const metricBounds = await page.locator("#metrics").boundingBox();
  assert.ok(waveformBounds && metricBounds, "waveform metrics have measurable bounds");
  assert.ok(
    metricBounds.x >= waveformBounds.x &&
      metricBounds.y >= waveformBounds.y &&
      metricBounds.x + metricBounds.width <= waveformBounds.x + waveformBounds.width &&
      metricBounds.y + metricBounds.height <= waveformBounds.y + waveformBounds.height,
    "RMS and peak must overlay the waveform without consuming a layout row",
  );
  assert.equal(
    await page.locator("#metrics").evaluate((metrics) =>
      getComputedStyle(metrics).backgroundColor
    ),
    "rgba(0, 0, 0, 0)",
    "waveform metrics must not show a mismatched box while loading",
  );
  assert.equal(await page.locator("#algorithmButtons button").count(), 7, "algorithm count");
  const uiScale = await page.evaluate(() => {
    const style = (selector) => getComputedStyle(document.querySelector(selector));
    return {
      buttonFont: style("#algorithmButtons button").fontSize,
      selectFont: style("#sourceABrowser .source-browser-trigger").fontSize,
      numberFont: style("#methodTools input[type='number']").fontSize,
      statusFont: style("#renderStatus").fontSize,
      metricFont: style("#metrics dd").fontSize,
      timeFont: style("#currentTime").fontSize,
      speedValueFont: style("#playbackSpeedValue").fontSize,
      toolLabelFont: style(".tool-control > span").fontSize,
      buttonHeight: style("#algorithmButtons button").height,
      selectHeight: style("#sourceABrowser .source-browser-trigger").height,
      sliderHeight: style("#methodTools input[type='range']").height,
    };
  });
  assert.deepEqual(uiScale, {
    buttonFont: "16px",
    selectFont: "16px",
    numberFont: "16px",
    statusFont: "16px",
    metricFont: "16px",
    timeFont: "16px",
    speedValueFont: "16px",
    toolLabelFont: "16px",
    buttonHeight: "36px",
    selectHeight: "38px",
    sliderHeight: "18px",
  });
  await assertNoUndersizedText(page);
  await assertToolLabelsFit(page);
  await expectContains(page, "#metrics", "rms");
  await expectContains(page, "#metrics", "peak");
  assert.deepEqual(
    await page.locator("#waveform, #spectrogram").evaluateAll((canvases) =>
      canvases.map((canvas) => ({
        fontSize: canvas.dataset.loadingFontSize,
        alignment: canvas.dataset.loadingTextAlignment,
      }))
    ),
    [
      { fontSize: "16", alignment: "center" },
      { fontSize: "16", alignment: "center" },
    ],
    "canvas loading text uses the centered 16px treatment",
  );
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
  assert.equal(await page.evaluate(() => transportSnapshot().loop), true);
  await assertAudioReady(page, "/windowed_convolution/5.00x5.00");
  const identityValidation = await page.evaluate(() => {
    const request = window.__CONV9_TEST_REQUESTS__.at(-1);
    const base = {
      renderEpoch: request.renderEpoch,
      requestId: request.requestId,
      leftId: request.leftId,
      rightId: request.rightId,
      algorithm: request.algorithm,
      windows: structuredClone(request.windows),
      parameters: structuredClone(request.parameters),
    };
    const wav = new Uint8Array(44);
    wav.set(new TextEncoder().encode("RIFF"), 0);
    wav.set(new TextEncoder().encode("WAVE"), 8);
    const mutations = [
      (header) => { header.renderEpoch++; },
      (header) => { header.requestId++; },
      (header) => { header.leftId += "_stale"; },
      (header) => { header.algorithm = "dry_a"; },
      (header) => { header.windows.clip_a_seconds += 1; },
      (header) => { header.parameters.input_taper += 0.1; },
      (header) => { delete header.parameters; },
    ];
    return mutations.map((mutate) => {
      const header = structuredClone(base);
      mutate(header);
      try {
        validateRenderedSelection(header, wav, request);
        return "";
      } catch (error) {
        return error.message;
      }
    });
  });
  assert.equal(identityValidation.length, 7);
  assert.ok(
    identityValidation.every((message) => message.includes("renderer returned")),
    `stale response identity was accepted: ${JSON.stringify(identityValidation)}`,
  );
  await assertCanvasHasVariation(page, "#waveform");
  await assertCanvasHasVariation(page, "#spectrogram");
  assert.equal(await page.locator("#spectrogram").getAttribute("data-fft-size"), "16384");
  assert.ok(
    Number(await page.locator("#spectrogram").getAttribute("data-analysis-columns")) > 720,
    "spectrogram time resolution exceeds the old 720-column cap",
  );
  const spectrumPerformance = await page.locator("#spectrogram").evaluate((canvas) => ({
    workers: Number(canvas.dataset.spectrumWorkers),
    visibleBins: Number(canvas.dataset.visibleBins),
    workerWallMs: Number(canvas.dataset.workerWallMs),
    workerComputeMs: Number(canvas.dataset.workerComputeMs),
    backingRows: canvas.height,
    log: state.performanceLog.at(-1),
  }));
  assert.ok(spectrumPerformance.workers >= 2, "spectrum analysis uses multiple workers");
  assert.ok(
    spectrumPerformance.visibleBins < spectrumPerformance.backingRows,
    "spectrum workers calculate each visible log-frequency bin only once",
  );
  assert.ok(
    Number.isFinite(spectrumPerformance.workerWallMs) &&
      spectrumPerformance.workerWallMs > 0 &&
      Number.isFinite(spectrumPerformance.log?.spectrumMs),
    `spectrum stage timing is logged: ${JSON.stringify(spectrumPerformance)}`,
  );
  console.log(
    `spectrum ${spectrumPerformance.workerWallMs.toFixed(1)} ms wall, ` +
      `${spectrumPerformance.workerComputeMs.toFixed(1)} ms aggregate, ` +
      `${spectrumPerformance.workers} workers`,
  );
  await assertNoViewportOverflow(page);
  await assertControlTooltips(page);
  assert.deepEqual(
    await page.evaluate(() => ({
      document: document.scrollingElement?.scrollTop ?? 0,
      body: document.body.scrollTop,
      shell: document.querySelector(".shell")?.scrollTop ?? 0,
    })),
    { document: 0, body: 0, shell: 0 },
    "tooltip inspection must not displace the fixed viewport",
  );
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
  await page.locator("#sourceBSelect").evaluate((select, value) => {
    select.value = value;
    select.dispatchEvent(new Event("change"));
  }, thirdSource);
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

  await page.getByRole("button", { name: "full", exact: true }).click();
  await waitForPath(page, "/full_convolution/a0.00+61.00_b0.00+61.00");
  assert.equal(await page.locator("#methodTools .window-control").count(), 0);
  assert.equal(await page.locator("#methodTools .tool-control").count(), 4);
  assert.equal(await page.locator("#methodTools input").count(), 8);
  assert.equal(await page.getByLabel("A offset exact value").inputValue(), "0.0");
  assert.equal(await page.getByLabel("A duration exact value").inputValue(), "61.0");
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

  await page.getByRole("button", { name: "dry a", exact: true }).click();
  await waitForPath(page, "/dry_a/source");
  assert.equal(await page.locator("#methodTools .window-control").count(), 0);
  assert.equal(await page.locator("#methodTools .tool-control").count(), 0);
  await expectContains(page, "#methodTools", "no configurable parameters");

  await page.getByRole("button", { name: "dry b", exact: true }).click();
  await waitForPath(page, "/dry_b/source");

  await page.getByRole("button", { name: "Play" }).click();
  await page.waitForFunction(() => transportSnapshot().playing);
  const beforeSwitch = await audioTime(page);
  await page.getByRole("button", { name: "chunks", exact: true }).click();
  const duringSwitch = await page.evaluate(() => transportSnapshot());
  assert.equal(duringSwitch.paused, true, "old audio stops as soon as a new render is queued");
  assert.equal(duringSwitch.readyState, 0, "transport is unavailable during a render");
  assert.equal(
    duringSwitch.desiredPlaying,
    true,
    "playing intent is retained without playing stale audio",
  );
  assert.ok(
    Math.abs(duringSwitch.currentTime - beforeSwitch) < 0.15,
    "render transition freezes the absolute playback position",
  );
  await page.waitForFunction(
    () => document.querySelector("#renderStatus")?.textContent === "rendering…",
  );
  await page.getByRole("button", { name: "vocoder", exact: true }).click();
  await waitForPath(
    page,
    "/source_filter_vocoder/vocoder_envelope_width_hz=900.00,vocoder_transfer=0.85,vocoder_transient_protection=0.65",
  );
  assert.equal(await page.locator("#methodTools .window-control").count(), 0);
  assert.equal(await page.locator("#methodTools .tool-control").count(), 3);
  await page.getByLabel("transfer exact value").fill("1.10");
  await waitForPath(page, "vocoder_transfer=1.10");
  assert.ok((await audioTime(page)) >= beforeSwitch - 0.5, "switch preserves position");
  assert.equal(await page.evaluate(() => transportSnapshot().paused), false);
  assert.match(await page.locator("#playButton").getAttribute("title"), /Pause playback/);
  const requests = await page.evaluate(() => window.__CONV9_TEST_REQUESTS__);
  assert.equal(
    requests.at(-1).algorithm,
    "source_filter_vocoder",
    "latest rapid selection wins",
  );
  await page.getByRole("button", { name: "resonators", exact: true }).click();
  await waitForPath(
    page,
    "/predictive_resonator_bank/resonator_ring=0.75,resonator_transfer=1.00",
  );
  assert.equal(await page.locator("#methodTools .window-control").count(), 0);
  assert.equal(await page.locator("#methodTools .tool-control").count(), 2);
  await page.getByLabel("transfer exact value").fill("0.72");
  await waitForPath(page, "resonator_transfer=0.72");
  assert.equal(
    await page.evaluate(() => window.__CONV9_TEST_REQUESTS__.at(-1).algorithm),
    "predictive_resonator_bank",
    "resonator selection retains its catalog parameters and request identity",
  );

  await page.getByRole("button", { name: "Pause" }).click();
  await page.locator("body").press("Space");
  await page.waitForFunction(() => transportSnapshot().playing);
  await page.locator("body").press("Space");
  await page.waitForFunction(() => transportSnapshot().paused);
  await page.getByRole("button", { name: "Play" }).click();
  await page.waitForTimeout(150);
  const pauseBoundary = await page.evaluate(() => ({
    presentation: transportCurrentTime(),
    render: transportRenderTime(state.transport.context.currentTime + 0.005),
    paused: (() => {
      state.transport.desiredPlaying = false;
      stopTransport(true);
      return transportSnapshot();
    })(),
  }));
  assert.ok(
    circularDistance(
      pauseBoundary.paused.currentTime,
      pauseBoundary.render,
      pauseBoundary.paused.duration,
    ) < 0.005,
    "pause resumes from the graph render head rather than replaying queued audio",
  );
  assert.ok(
    circularDistance(
      pauseBoundary.paused.currentTime,
      pauseBoundary.presentation,
      pauseBoundary.paused.duration,
    ) < 0.08,
    "render-head preservation stays within a small output-latency interval",
  );
  const rapidToggle = await page.evaluate(async () => {
    await state.transport.context.suspend();
    const play = togglePlayback();
    const pause = togglePlayback();
    await Promise.allSettled([play, pause]);
    return transportSnapshot();
  });
  assert.equal(rapidToggle.paused, true, "a pause cancels an in-flight play request");
  assert.equal(rapidToggle.desiredPlaying, false, "rapid play/pause leaves no stale autoplay intent");
  assert.equal(
    await page.getByRole("button", { name: "Play" }).count(),
    1,
    "button reflects cancelled play intent",
  );

  await page.locator("#volume").evaluate((input) => {
    input.value = "0.37";
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  assert.equal(await page.evaluate(() => transportSnapshot().volume), 0.37);
  assert.equal(await page.evaluate(() => transportSnapshot().playbackRate), 1);
  assert.deepEqual(
    await page.locator("#playbackSpeed").evaluate((input) => ({
      minimum: input.min,
      maximum: input.max,
      step: input.step,
      value: input.value,
      normalizedPosition: (Number(input.value) - Number(input.min)) /
        (Number(input.max) - Number(input.min)),
    })),
    {
      minimum: "-1",
      maximum: "1",
      step: "any",
      value: "0",
      normalizedPosition: 0.5,
    },
  );
  await page.locator("#playbackSpeed").evaluate((input) => {
    input.value = "0.5";
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  assert.ok(
    Math.abs(
      (await page.evaluate(() => transportSnapshot().playbackRate)) - 1.5,
    ) < 1e-6,
  );
  assert.ok(
    Math.abs(
      Number(await page.locator("#playbackSpeed").inputValue()) - Math.log2(1.5),
    ) < 1e-6,
    "slider thumb snaps to the logarithmic position for 1.5×",
  );
  assert.equal(await page.locator("#playbackSpeedValue").textContent(), "1.50×");
  assert.equal(
    await page.locator("#playbackSpeed").getAttribute("aria-valuetext"),
    "1.50 times",
  );
  await page.locator("#playbackSpeed").press("ArrowRight");
  assert.equal(await page.locator("#playbackSpeedValue").textContent(), "1.75×");
  await page.locator("#playbackSpeed").press("ArrowLeft");
  assert.equal(await page.locator("#playbackSpeedValue").textContent(), "1.50×");
  await page.locator("#playbackSpeed").evaluate((input) => input.blur());
  assert.equal(
    await page.locator("#preservePitch").evaluate((input) =>
      input.parentElement.previousElementSibling?.classList.contains("playback-speed")
    ),
    true,
    "pitch preservation sits directly next to playback speed",
  );
  assert.deepEqual(
    await page.locator("#preservePitch").evaluate((input) => {
      const style = getComputedStyle(input);
      return {
        width: style.width,
        height: style.height,
        borderWidth: style.borderWidth,
        backgroundImage: style.backgroundImage,
      };
    }),
    {
      width: "18px",
      height: "18px",
      borderWidth: "1px",
      backgroundImage: "none",
    },
    "unchecked pitch control is a visible bordered square without a stray glyph",
  );
  await page.locator("#preservePitch").check();
  assert.equal(await page.evaluate(() => transportSnapshot().preservePitch), true);
  assert.match(
    await page.locator("#preservePitch").evaluate((input) =>
      getComputedStyle(input).backgroundImage
    ),
    /svg/,
    "checked pitch control uses the intended SVG check glyph",
  );
  if (process.env.CONV9_TEST_SCREENSHOT) {
    await page.screenshot({
      path: `${process.env.CONV9_TEST_SCREENSHOT}.pitch.png`,
    });
  }
  await page.getByRole("button", { name: "Play" }).click();
  await page.waitForFunction(
    () =>
      transportSnapshot().playing &&
      transportSnapshot().pitchLatency > 0.03 &&
      state.transport.sourceEffect instanceof AudioWorkletNode,
  );
  await page.waitForTimeout(180);
  assert.ok(
    (await page.evaluate(() => transportSnapshot().currentTime)) > 0.1,
    "pitch-preserved transport advances at the selected listening speed",
  );
  await page.getByRole("button", { name: "Pause" }).click();
  await page.locator("#preservePitch").uncheck();
  assert.equal(await page.evaluate(() => transportSnapshot().preservePitch), false);
  await page.locator("#preservePitch").evaluate((input) => input.blur());

  await page.locator("#seek").evaluate((input) => {
    input.value = "11";
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await expectAudioTime(page, 11, 0.2);
  await page.locator("body").press("ArrowRight");
  await expectAudioTime(page, 16, 0.2);
  await page.locator("body").press("ArrowLeft");
  await expectAudioTime(page, 11, 0.2);

  await setAudioTime(page, 61);
  const endpoint = await page.evaluate(() => transportSnapshot());
  assert.ok(endpoint.currentTime < endpoint.duration, "endpoint seeks clamp to a playable sample");
  assert.ok(
    endpoint.duration - endpoint.currentTime < 0.001,
    "endpoint clamp stays within one millisecond of the final sample",
  );
  await page.getByRole("button", { name: "Play" }).click();
  await page.waitForTimeout(900);
  const afterLoop = await page.evaluate(() => ({
    transport: transportSnapshot(),
    label: document.querySelector("#currentTime").textContent,
    seek: Number(document.querySelector("#seek").value),
  }));
  assert.ok(
    afterLoop.transport.currentTime > 0.2 && afterLoop.transport.currentTime < 2,
    `sample-clock loop did not wrap cleanly: ${JSON.stringify(afterLoop)}`,
  );
  assert.ok(
    Math.abs(parseTimeLabel(afterLoop.label) - afterLoop.transport.currentTime) < 0.05,
    "numeric time follows the same loop clock as playback",
  );
  assert.ok(
    Math.abs(afterLoop.seek - afterLoop.transport.currentTime) < 0.05,
    "seek indicator follows the same loop clock as playback",
  );
  await page.getByRole("button", { name: "Pause" }).click();

  const waveform = page.locator("#waveform");
  const bounds = await waveform.boundingBox();
  assert.ok(bounds, "waveform is visible");
  await waveform.click({ position: { x: bounds.width * 0.75, y: bounds.height / 2 } });
  await expectAudioTime(page, 45.75, 0.5);

  const sourceBeforeResize = await audioSource(page);
  const timeBeforeResize = await audioTime(page);
  const analysisCountBeforeResize = await page.evaluate(() => state.performanceLog.length);
  await page.setViewportSize({ width: 900, height: 640 });
  await page.waitForTimeout(1_500);
  assert.equal(await audioSource(page), sourceBeforeResize, "resize does not reload audio");
  await expectAudioTime(page, timeBeforeResize, 0.2);
  await assertCanvasHasVariation(page, "#waveform");
  await assertCanvasHasVariation(page, "#spectrogram");
  await assertNoViewportOverflow(page);
  await assertNoUndersizedText(page);
  await assertToolLabelsFit(page);
  assert.equal(
    await page.evaluate(() => state.performanceLog.length),
    analysisCountBeforeResize,
    "resize rescales the cached spectrum instead of running another FFT analysis",
  );
  assert.equal(await page.locator("#metrics").isVisible(), true, "metrics remain on compact row");
  await page.locator("#sourceBBrowser .source-browser-trigger").click();
  await page.locator("#source-b-dialog").waitFor({ state: "visible" });
  await page.waitForFunction(
    () =>
      document.querySelector("#source-b-dialog .source-preview-waveform")
        ?.getAttribute("aria-busy") === "false",
  );
  const compactDialogBounds = await page.locator("#source-b-dialog").boundingBox();
  assert.ok(
    compactDialogBounds?.width >= 860 && compactDialogBounds?.height >= 520,
    `compact source browser should fill the usable viewport: ${JSON.stringify(compactDialogBounds)}`,
  );
  await assertCanvasMatchesLayout(page, "#source-b-dialog .source-preview-waveform");
  await assertNoViewportOverflow(page);
  await assertNoUndersizedText(page);
  if (process.env.CONV9_TEST_SCREENSHOT) {
    await page.screenshot({ path: `${process.env.CONV9_TEST_SCREENSHOT}.compact.png` });
  }
  await page.locator("#source-b-dialog .source-search").press("Escape");

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
      vocoder_transfer:
        "Sets how strongly clip B's smoothed spectral envelope reshapes clip A.",
      vocoder_envelope_width_hz:
        "Sets the frequency span used to smooth both short-time spectra before transfer.",
      vocoder_transient_protection:
        "Reduces envelope transfer during rapid spectral onsets from clip A.",
      resonator_transfer:
        "Moves clip B from its own response toward clip A's learned resonances while preserving B's innovation, events, and timeline.",
      resonator_ring:
        "Controls damping of clip A's learned stable resonances; higher values retain narrower modes and longer ringing.",
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
    schema_version: 9,
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
        id: "source_filter_vocoder",
        title: "Source-filter vocoder",
        description:
          "Uses A for excitation, phase, and timing while B supplies a smooth short-time spectral envelope.",
        rank: 2,
        windows: [],
        parameters: [
          parameter("vocoder_transfer", "transfer", 0, 1.5, 0.01, 0.85),
          parameter(
            "vocoder_envelope_width_hz",
            "envelope width",
            100,
            3000,
            50,
            900,
            "Hz",
          ),
          parameter(
            "vocoder_transient_protection",
            "transients",
            0,
            1,
            0.01,
            0.65,
          ),
        ],
      },
      {
        id: "predictive_resonator_bank",
        title: "Predictive resonator bank",
        description:
          "Learns stable resonances from A, recovers B's innovation signal, and drives A's acoustic body with B's events and timeline.",
        rank: 3,
        windows: [],
        parameters: [
          parameter("resonator_transfer", "transfer", 0, 1, 0.01, 1),
          parameter("resonator_ring", "ring", 0, 1, 0.01, 0.75),
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
        rank: 5,
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
        rank: 6,
        windows: [],
        parameters: [],
      },
      {
        id: "dry_b",
        title: "Dry source B",
        description:
          "Plays the complete conditioned clip B without convolution, output saturation, or a second level-normalization pass.",
        rank: 7,
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
    const waveform = document.querySelector("#waveform");
    const spectrum = document.querySelector("#spectrogram");
    return (
      transportSnapshot().readyState >= 4 &&
      waveform?.getAttribute("aria-busy") === "false" &&
      spectrum?.getAttribute("aria-busy") === "false" &&
      document.querySelector("#renderStatus")?.textContent.startsWith("rendered ")
    );
  }, { timeout: 120_000 });
}

async function waitForPath(page, pathPart) {
  await page.waitForFunction(
    (part) =>
      transportSnapshot().path.includes(part) &&
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
      const transport = transportSnapshot();
      return (
        transport.readyState >= 4 &&
        Math.abs(transport.duration - 61) < 0.01 &&
        transport.path.includes(part)
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

async function canvasFingerprint(page, selector) {
  return page.locator(selector).evaluate((canvas) => {
    const pixels = canvas
      .getContext("2d", { willReadFrequently: true })
      .getImageData(0, 0, canvas.width, canvas.height).data;
    let hash = 2166136261;
    for (let index = 0; index < pixels.length; index += 37) {
      hash ^= pixels[index];
      hash = Math.imul(hash, 16777619);
    }
    return hash >>> 0;
  });
}

async function assertCanvasMatchesLayout(page, selector) {
  const dimensions = await page.locator(selector).evaluate((canvas) => {
    const bounds = canvas.getBoundingClientRect();
    const scale = Math.min(2, Math.max(1, window.devicePixelRatio || 1));
    return {
      bitmapWidth: canvas.width,
      bitmapHeight: canvas.height,
      expectedWidth: Math.round(bounds.width * scale),
      expectedHeight: Math.round(bounds.height * scale),
    };
  });
  assert.ok(
    Math.abs(dimensions.bitmapWidth - dimensions.expectedWidth) <= 1 &&
      Math.abs(dimensions.bitmapHeight - dimensions.expectedHeight) <= 1,
    `canvas bitmap is stretched away from its layout size: ${JSON.stringify(dimensions)}`,
  );
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
  await page.evaluate((time) => seekTransport(time), seconds);
  await expectAudioTime(page, seconds, 0.2);
}

async function expectAudioTime(page, expected, tolerance) {
  await page.waitForFunction(
    ({ target, allowed }) =>
      Math.abs(transportSnapshot().currentTime - target) <= allowed,
    { target: expected, allowed: tolerance },
  );
}

function audioTime(page) {
  return page.evaluate(() => transportSnapshot().currentTime);
}

function audioSource(page) {
  return page.evaluate(() => transportSnapshot().path);
}

function parseTimeLabel(label) {
  const [minutes, seconds] = label.split(":");
  return Number(minutes) * 60 + Number(seconds);
}

function circularDistance(left, right, duration) {
  const distance = Math.abs(left - right);
  return Math.min(distance, Math.max(0, duration - distance));
}
