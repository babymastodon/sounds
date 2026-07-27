import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { readFile, unlink } from "node:fs/promises";
import { createConnection, createServer } from "node:net";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const appDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const conv9Dir = resolve(appDir, "..");
const windowTitle = "Convolution Playground";
const application = resolve(appDir, "src-tauri/target/debug/conv9-listener");
const manifest = resolve(conv9Dir, "sources.tsv");
const inputDir = resolve(conv9Dir, "samples/prepared");
const retiredOutputDir = resolve(conv9Dir, "outputs");
const iconSvg = resolve(appDir, "src-tauri/icons/icon.svg");
const iconPng = resolve(appDir, "src-tauri/icons/icon.png");
const driverBinary =
  process.env.TAURI_DRIVER_BIN || "/tmp/conv9-tauri-driver/bin/tauri-driver";
const sysroot = process.env.CONV9_TAURI_SYSROOT || "/tmp/conv9-tauri-devel";
const sinkName = `conv9_test_${process.pid}`;
const capturePath = `/tmp/${sinkName}.wav`;
const firstStartCapturePath = `/tmp/${sinkName}_first-start.wav`;
const replayStartCapturePath = `/tmp/${sinkName}_replay-start.wav`;

assert.ok(existsSync(application), `build the Tauri app first: missing ${application}`);
assert.ok(
  existsSync(driverBinary),
  `install tauri-driver or set TAURI_DRIVER_BIN: missing ${driverBinary}`,
);
assert.ok(existsSync(manifest), "conv9 source manifest is missing");
assert.ok(existsSync(resolve(inputDir, "ambient_guitar.wav")), "prepared inputs are missing");
assert.equal(existsSync(retiredOutputDir), false, "precomputed output tree must remain absent");
assert.ok(existsSync(iconSvg), "editable SVG app icon is missing");
assert.ok(existsSync(iconPng), "embedded PNG app icon is missing");

let moduleId;
let driver;
let sessionId;
const driverOutput = [];

try {
  moduleId = loadNullSink();
  const port = await availablePort();
  const nativePort = await availablePort();
  driver = spawn(
    driverBinary,
    ["--port", String(port), "--native-port", String(nativePort)],
    {
      env: nativeEnvironment(),
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  driver.stdout.on("data", (chunk) => driverOutput.push(chunk.toString()));
  driver.stderr.on("data", (chunk) => driverOutput.push(chunk.toString()));
  await waitForPort(port, driver);

  const session = await webdriver(port, "POST", "/session", {
    capabilities: {
      alwaysMatch: {
        browserName: "wry",
        "tauri:options": { application },
      },
    },
  });
  sessionId = session.sessionId;
  assert.ok(sessionId, "native WebDriver did not return a session id");

  const initial = await poll(async () => {
    const value = await execute(port, sessionId, `
      const transport = transportSnapshot();
      const waveform = document.querySelector("#waveform");
      const spectrum = document.querySelector("#spectrogram");
      return {
        readyState: transport.readyState,
        documentTitle: document.title,
        duration: transport.duration,
        path: transport.path,
        loop: transport.loop,
        playbackRate: transport.playbackRate,
        renderEpoch: state.renderEpoch,
        contextState: transport.contextState,
        bufferRevision: transport.bufferRevision,
        mediaElementCount: document.querySelectorAll("audio, video").length,
        playbackSpeedValue: document.querySelector("#playbackSpeedValue")?.textContent,
        playbackSpeedScale: {
          minimum: document.querySelector("#playbackSpeed")?.min,
          maximum: document.querySelector("#playbackSpeed")?.max,
          value: document.querySelector("#playbackSpeed")?.value
        },
        status: document.querySelector("#renderStatus")?.textContent,
        statusPosition: getComputedStyle(document.querySelector("#renderStatus")).position,
        sourceACount: document.querySelector("#sourceASelect")?.options.length,
        sourceBCount: document.querySelector("#sourceBSelect")?.options.length,
        sourceA: document.querySelector("#sourceASelect")?.value,
        sourceB: document.querySelector("#sourceBSelect")?.value,
        methodCount: document.querySelectorAll("#algorithmButtons button").length,
        methodHeaderCount: document.querySelectorAll(
          "#methodToolTitle, .method-panel > header"
        ).length,
        appHeaderCaptionCount: document.querySelectorAll(
          "h1, .method-field, .field-label"
        ).length,
        repeatedDetailCount: document.querySelectorAll(
          "#renderTitle, #windowReadout, .visual-card > header, .now-playing"
        ).length,
        metricsVisible: getComputedStyle(document.querySelector("#metrics")).display !== "none",
        metricsText: document.querySelector("#metrics")?.textContent,
        metricsOverlay: (() => {
          const metrics = document.querySelector("#metrics").getBoundingClientRect();
          const waveform = document.querySelector("#waveform").getBoundingClientRect();
          return {
            inside:
              metrics.left >= waveform.left &&
              metrics.top >= waveform.top &&
              metrics.right <= waveform.right &&
              metrics.bottom <= waveform.bottom,
            position: getComputedStyle(document.querySelector("#metrics")).position,
            background: getComputedStyle(document.querySelector("#metrics")).backgroundColor
          };
        })(),
        loadingTextStyles: [
          {
            fontSize: waveform?.dataset.loadingFontSize,
            alignment: waveform?.dataset.loadingTextAlignment
          },
          {
            fontSize: spectrum?.dataset.loadingFontSize,
            alignment: spectrum?.dataset.loadingTextAlignment
          }
        ],
        windowCount: document.querySelectorAll("#methodTools .window-control").length,
        playDisabled: document.querySelector("#playButton")?.disabled,
        seekDisabled: document.querySelector("#seek")?.disabled,
        uiScale: {
          buttonFont: getComputedStyle(
            document.querySelector("#algorithmButtons button")
          ).fontSize,
          selectFont: getComputedStyle(
            document.querySelector("#sourceABrowser .source-browser-trigger")
          ).fontSize,
          numberFont: getComputedStyle(
            document.querySelector("#methodTools input[type='number']")
          ).fontSize,
          statusFont: getComputedStyle(document.querySelector("#renderStatus")).fontSize,
          metricFont: getComputedStyle(document.querySelector("#metrics dd")).fontSize,
          timeFont: getComputedStyle(document.querySelector("#currentTime")).fontSize,
          speedValueFont: getComputedStyle(
            document.querySelector("#playbackSpeedValue")
          ).fontSize,
          toolLabelFont: getComputedStyle(
            document.querySelector(".tool-control > span")
          ).fontSize,
          buttonHeight: getComputedStyle(
            document.querySelector("#algorithmButtons button")
          ).height,
          selectHeight: getComputedStyle(
            document.querySelector("#sourceABrowser .source-browser-trigger")
          ).height,
          sliderHeight: getComputedStyle(
            document.querySelector("#methodTools input[type='range']")
          ).height
        },
        undersizedFonts: [...document.querySelectorAll("body *")]
          .filter((element) => {
            const style = getComputedStyle(element);
            if (
              style.display === "none" ||
              style.visibility === "hidden" ||
              element.getClientRects().length === 0
            ) return false;
            const hasOwnText = [...element.childNodes].some(
              (node) => node.nodeType === Node.TEXT_NODE && node.textContent.trim()
            );
            const isTextControl = element.matches("button, select, input, output");
            return (hasOwnText || isTextControl) && parseFloat(style.fontSize) < 16;
          })
          .map((element) =>
            element.tagName.toLowerCase() + "#" +
            (element.id || element.className || element.getAttribute("aria-label"))
          ),
        missingTooltips: [...document.querySelectorAll("button, select, input, canvas, a[href]")]
          .filter((control) =>
            !control.title ||
            control.title.trim().length < 40 ||
            control.title.includes("undefined")
          )
          .map((control) => control.id || control.getAttribute("aria-label") || control.textContent),
        fftSize: spectrum?.dataset.fftSize,
        error: document.querySelector("#errorPanel")?.textContent,
        errorHidden: document.querySelector("#errorPanel")?.hidden,
        waveformReady: waveform?.getAttribute("aria-busy") === "false",
        spectrumReady: spectrum?.getAttribute("aria-busy") === "false",
        viewport: {
          width: innerWidth,
          height: innerHeight,
          scrollWidth: document.documentElement.scrollWidth,
          scrollHeight: document.documentElement.scrollHeight
        }
      };
    `);
    return value.readyState >= 4 &&
      value.waveformReady &&
      value.spectrumReady &&
      value.status?.startsWith("rendered ") &&
      !value.playDisabled
      ? value
      : undefined;
  }, 180_000);
  assert.ok(
    Math.abs(initial.duration - 132) < 0.02,
    `default windowed-convolution duration was ${initial.duration}`,
  );
  assert.equal(initial.documentTitle, windowTitle);
  assert.equal(
    await webdriver(port, "GET", `/session/${sessionId}/title`),
    windowTitle,
    "native window title",
  );
  const embeddedIcon = await executeAsync(port, sessionId, `
    const done = arguments[arguments.length - 1];
    window.__TAURI__.app.defaultWindowIcon()
      .then(async (icon) => {
        if (!icon) {
          done(null);
          return;
        }
        const size = await icon.size();
        const rgba = await icon.rgba();
        done({
          size,
          byteLength: rgba.byteLength,
          firstPixel: [...rgba.slice(0, 4)]
        });
      })
      .catch((error) => done({ error: String(error) }));
  `);
  assert.deepEqual(
    embeddedIcon,
    {
      size: { width: 512, height: 512 },
      byteLength: 512 * 512 * 4,
      firstPixel: [23, 27, 26, 255],
    },
    "Tauri must embed the 512px convolution icon",
  );
  assert.match(initial.path, /windowed_convolution\/5\.00x5\.00\//);
  assert.equal(initial.loop, true);
  assert.equal(initial.mediaElementCount, 0, "playback must not use native media-element queues");
  assert.equal(initial.bufferRevision, 1);
  assert.equal(initial.playbackRate, 1);
  assert.equal(initial.playbackSpeedValue, "1.00×");
  assert.deepEqual(initial.playbackSpeedScale, {
    minimum: "-1",
    maximum: "1",
    value: "0",
  });
  assert.match(initial.status, /^rendered \d+ ms$/);
  assert.equal(initial.statusPosition, "absolute");
  assert.equal(initial.sourceACount, 96);
  assert.equal(initial.sourceBCount, 96);
  assert.equal(initial.sourceA, "gamelan_court");
  assert.equal(initial.sourceB, "chainsaw_cycle");
  assert.equal(initial.methodCount, 8);
  assert.equal(initial.methodHeaderCount, 0);
  assert.equal(initial.appHeaderCaptionCount, 0);
  assert.equal(initial.repeatedDetailCount, 0);
  assert.equal(initial.metricsVisible, true);
  assert.match(initial.metricsText, /rms/);
  assert.match(initial.metricsText, /peak/);
  assert.deepEqual(initial.metricsOverlay, {
    inside: true,
    position: "absolute",
    background: "rgba(0, 0, 0, 0)",
  });
  assert.deepEqual(initial.loadingTextStyles, [
    { fontSize: "16", alignment: "center" },
    { fontSize: "16", alignment: "center" },
  ]);
  assert.equal(initial.windowCount, 2);
  assert.equal(initial.playDisabled, false);
  assert.equal(initial.seekDisabled, false);
  assert.deepEqual(initial.uiScale, {
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
  assert.deepEqual(initial.undersizedFonts, []);
  assert.deepEqual(initial.missingTooltips, []);
  assert.equal(initial.fftSize, "16384");
  assert.equal(initial.errorHidden, true, initial.error);
  assert.ok(initial.viewport.scrollWidth <= initial.viewport.width, "native horizontal overflow");
  assert.ok(initial.viewport.scrollHeight <= initial.viewport.height, "native vertical overflow");

  await execute(port, sessionId, `
    document.querySelector("#sourceABrowser .source-browser-trigger").click();
    return true;
  `);
  const nativePreview = await poll(async () => {
    const value = await execute(port, sessionId, `
      const dialog = document.querySelector("#source-a-dialog");
      const canvas = dialog.querySelector(".source-preview-waveform");
      const spectrumCanvas = dialog.querySelector(".source-preview-spectrum");
      const canvasVariation = (target) => {
        const pixels = target.getContext("2d").getImageData(
          0, 0, target.width, target.height
        ).data;
        let variation = 0;
        for (let index = 4; index < pixels.length; index += 97) {
          variation += Math.abs(pixels[index] - pixels[index - 4]);
        }
        return variation;
      };
      const bounds = dialog.getBoundingClientRect();
      const preview = dialog.querySelector(".source-preview").getBoundingClientRect();
      const plots = dialog.querySelector(".source-preview-plots").getBoundingClientRect();
      const plotContainment = [...dialog.querySelectorAll(".source-preview-plot")]
        .map((frame) => {
          const frameBounds = frame.getBoundingClientRect();
          const canvasBounds = frame.querySelector("canvas").getBoundingClientRect();
          return frameBounds.top >= plots.top && frameBounds.bottom <= plots.bottom &&
            canvasBounds.top >= frameBounds.top &&
            canvasBounds.bottom <= frameBounds.bottom &&
            getComputedStyle(frame).overflow === "hidden";
        });
      const trigger = document.querySelector(
        "#sourceABrowser .source-browser-trigger"
      ).getBoundingClientRect();
      const caret = document.querySelector(
        "#sourceABrowser .source-trigger-chevron"
      ).getBoundingClientRect();
      return {
        open: !dialog.hidden,
        busy: canvas.getAttribute("aria-busy"),
        stats: dialog.querySelector(".source-preview-stats").textContent,
        waveformVariation: canvasVariation(canvas),
        spectrumVariation: canvasVariation(spectrumCanvas),
        spectrumColumns: spectrumCanvas.dataset.spectrumColumns,
        spectrumRows: spectrumCanvas.dataset.spectrumRows,
        previewFftSize: spectrumCanvas.dataset.fftSize,
        plotsContained:
          plots.top >= preview.top && plots.bottom <= preview.bottom &&
          plotContainment.length === 2 && plotContainment.every(Boolean),
        subtitleCount: dialog.querySelectorAll(".source-preview-kind").length,
        rowSubtitleCount: dialog.querySelectorAll(".source-option-kind").length,
        caretOffset: Math.abs(
          (trigger.top + trigger.bottom) / 2 - (caret.top + caret.bottom) / 2
        ),
        inViewport:
          bounds.left >= 0 && bounds.top >= 0 &&
          bounds.right <= innerWidth && bounds.bottom <= innerHeight,
        expanded:
          document.querySelector("#sourceABrowser .source-browser-trigger")
            .getAttribute("aria-expanded")
      };
    `);
    return value.open && value.busy === "false" ? value : undefined;
  }, 30_000);
  assert.match(nativePreview.stats, /rms/);
  assert.match(nativePreview.stats, /peak/);
  assert.ok(nativePreview.waveformVariation > 0, "native source preview waveform must vary");
  assert.ok(nativePreview.spectrumVariation > 0, "native source preview spectrum must vary");
  assert.equal(nativePreview.spectrumColumns, "420", "native FFT map preserves time slices");
  assert.equal(nativePreview.spectrumRows, "192", "native FFT map preserves frequency rows");
  assert.equal(nativePreview.previewFftSize, "8192", "native preview uses the higher FFT size");
  assert.equal(nativePreview.plotsContained, true, "native plots stay inside preview");
  assert.equal(nativePreview.subtitleCount, 0, "native preview has no subtitle");
  assert.equal(nativePreview.rowSubtitleCount, 0, "native result rows have no subtitles");
  assert.ok(
    nativePreview.caretOffset <= 1,
    `native source caret is ${nativePreview.caretOffset}px off vertical center`,
  );
  assert.equal(nativePreview.inViewport, true, "native source browser stays inside viewport");
  assert.equal(nativePreview.expanded, "true");
  await execute(port, sessionId, `
    document.querySelector("#source-a-dialog .source-search")
      .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    return document.querySelector("#source-a-dialog").hidden;
  `);

  const renderedPrefix = await execute(port, sessionId, `
    const samples = state.analysisSamples;
    const sampleRate = state.analysisSampleRate;
    const windowFrames = Math.floor(sampleRate / 10);
    const rmsDb = [];
    for (let window = 0; window < 20; window++) {
      let power = 0;
      const start = window * windowFrames;
      const end = Math.min(samples.length, (window + 1) * windowFrames);
      for (let index = start; index < end; index++) {
        power += samples[index] * samples[index];
      }
      rmsDb.push(
        10 * Math.log10(Math.max(Number.EPSILON, power / Math.max(1, end - start)))
      );
    }
    const decimation = 12;
    const referenceFrames = Math.min(samples.length, sampleRate * 2);
    const reference = new Array(Math.floor(referenceFrames / decimation));
    for (let index = 0; index < reference.length; index++) {
      reference[index] = samples[index * decimation];
    }
    return { sampleRate, rmsDb, decimation, reference };
  `);
  assert.ok(
    renderedPrefix.rmsDb[0] < renderedPrefix.rmsDb[4] - 10 &&
      renderedPrefix.rmsDb[4] < renderedPrefix.rmsDb[19] - 10,
    `default render should fade in gradually: ${renderedPrefix.rmsDb
      .map((level) => level.toFixed(1))
      .join(" ")}`,
  );

  await capturePlaybackStart(port, sessionId, firstStartCapturePath);
  await resetPlayback(port, sessionId);
  await capturePlaybackStart(port, sessionId, replayStartCapturePath);
  const firstPlayComparison = compareCapturedToReference(
    await readFile(firstStartCapturePath),
    renderedPrefix.reference,
    renderedPrefix.decimation,
    renderedPrefix.sampleRate,
  );
  const replayComparison = compareCapturedToReference(
    await readFile(replayStartCapturePath),
    renderedPrefix.reference,
    renderedPrefix.decimation,
    renderedPrefix.sampleRate,
  );
  console.log(
    `first-play regression: ${firstPlayComparison.earlyCorrelation.toFixed(4)} early / ` +
      `${firstPlayComparison.lateCorrelation.toFixed(4)} late correlation, ` +
      `${firstPlayComparison.lagFrames} frame capture alignment`,
  );
  for (const [name, comparison] of [
    ["first play", firstPlayComparison],
    ["replay", replayComparison],
  ]) {
    assert.ok(
      comparison.prefixRmsDb[0] < -80,
      `${name} emitted a burst in its first 100 ms: ${comparison.prefixRmsDb.join(" ")}`,
    );
    assert.ok(
      Math.max(...comparison.prefixRmsDb) < -45,
      `${name} did not preserve the quiet 0–500 ms fade: ${comparison.prefixRmsDb.join(" ")}`,
    );
  }
  assert.ok(
    firstPlayComparison.earlyCorrelation > 0.98,
    `first playback does not match the decoded buffer: ${JSON.stringify(firstPlayComparison)}`,
  );
  assert.ok(
    firstPlayComparison.lateCorrelation > 0.98,
    `first-play continuation does not match the decoded buffer: ${JSON.stringify(firstPlayComparison)}`,
  );
  assert.ok(
    replayComparison.earlyCorrelation > 0.98 &&
      replayComparison.lateCorrelation > 0.98,
    `replay does not match the decoded buffer: ${JSON.stringify(replayComparison)}`,
  );
  assert.ok(
    Math.abs(firstPlayComparison.lagFrames - replayComparison.lagFrames) < 2_400,
    `first start and replay output latencies diverged: ${JSON.stringify({
      firstPlayComparison,
      replayComparison,
    })}`,
  );
  assert.ok(
    Math.abs(
      firstPlayComparison.earlyLagFrames - firstPlayComparison.lateLagFrames,
    ) < 480,
    `capture clock drifted by more than 10 ms: ${JSON.stringify(firstPlayComparison)}`,
  );
  await resetPlayback(port, sessionId);

  const changedPlaybackRate = await execute(port, sessionId, `
    const speed = document.querySelector("#playbackSpeed");
    speed.value = "0.5";
    speed.dispatchEvent(new Event("input", { bubbles: true }));
    const result = {
      rate: transportSnapshot().playbackRate,
      value: document.querySelector("#playbackSpeedValue").textContent
    };
    speed.value = "0";
    speed.dispatchEvent(new Event("input", { bubbles: true }));
    return result;
  `);
  assert.ok(Math.abs(changedPlaybackRate.rate - 1.5) < 1e-6);
  assert.equal(changedPlaybackRate.value, "1.50×");

  await execute(port, sessionId, `
    const speed = document.querySelector("#playbackSpeed");
    speed.value = "0.5";
    speed.dispatchEvent(new Event("input", { bubbles: true }));
    const pitch = document.querySelector("#preservePitch");
    pitch.checked = true;
    pitch.dispatchEvent(new Event("change", { bubbles: true }));
    return true;
  `);
  await poll(async () => {
    const value = await execute(port, sessionId, `
      return {
        checked: document.querySelector("#preservePitch").checked,
        disabled: document.querySelector("#preservePitch").disabled,
        loaded: Boolean(state.transport.pitchWorkletPromise),
        error: document.querySelector("#errorPanel")?.textContent,
        errorHidden: document.querySelector("#errorPanel")?.hidden
      };
    `);
    return value.checked && !value.disabled && value.loaded && value.errorHidden
      ? value
      : undefined;
  });
  await execute(
    port,
    sessionId,
    `document.querySelector("#playButton").click(); return true;`,
  );
  const pitchPreserved = await poll(async () => {
    const value = await execute(port, sessionId, `
      const transport = transportSnapshot();
      return {
        playing: transport.playing,
        rate: transport.playbackRate,
        preservePitch: transport.preservePitch,
        pitchLatency: transport.pitchLatency,
        effect: state.transport.sourceEffect?.constructor?.name,
        error: document.querySelector("#errorPanel")?.textContent,
        errorHidden: document.querySelector("#errorPanel")?.hidden
      };
    `);
    return value.playing && value.pitchLatency > 0.03 ? value : undefined;
  });
  assert.equal(pitchPreserved.preservePitch, true);
  assert.ok(Math.abs(pitchPreserved.rate - 1.5) < 1e-6);
  assert.equal(pitchPreserved.effect, "AudioWorkletNode");
  assert.equal(pitchPreserved.errorHidden, true, pitchPreserved.error);
  await captureMonitor();
  const pitchSignal = analyzePcm16(await readFile(capturePath));
  assert.ok(pitchSignal.rms > 0.003, `pitch-preserved native output is silent: ${pitchSignal.rms}`);
  assert.ok(pitchSignal.peak > 0.01, `pitch-preserved native output peak is too low: ${pitchSignal.peak}`);
  await resetPlayback(port, sessionId);
  await execute(port, sessionId, `
    const pitch = document.querySelector("#preservePitch");
    pitch.checked = false;
    pitch.dispatchEvent(new Event("change", { bubbles: true }));
    const speed = document.querySelector("#playbackSpeed");
    speed.value = "0";
    speed.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  `);

  await execute(
    port,
    sessionId,
    `document.querySelector("#playButton").click(); return true;`,
  );
  const playing = await poll(async () => {
    const value = await playbackState(port, sessionId);
    return !value.paused && value.currentTime > 0.5 ? value : undefined;
  });
  assert.equal(playing.errorHidden, true, playing.error);

  const routing = await poll(() => {
    const details = command("pactl", ["list", "sink-inputs"]);
    return details.includes('application.name = "conv9-listener"') &&
      details.includes(`target.object = "${sinkName}"`)
      ? details
      : undefined;
  });
  assert.match(routing, /Corked: no/);
  assert.match(routing, /Mute: no/);

  await captureMonitor();
  const signal = analyzePcm16(await readFile(capturePath));
  assert.ok(signal.samples >= 48_000, `capture too short: ${signal.samples} samples`);
  assert.ok(signal.rms > 0.003, `native output is silent: RMS ${signal.rms}`);
  assert.ok(signal.peak > 0.01, `native output peak is too low: ${signal.peak}`);

  const afterCapture = await playbackState(port, sessionId);
  assert.ok(
    afterCapture.currentTime > playing.currentTime + 2,
    "native playback did not advance during capture",
  );
  assert.equal(afterCapture.paused, false);
  assert.equal(afterCapture.errorHidden, true, afterCapture.error);

  await execute(port, sessionId, `
    const selectSource = (selector, value) => {
      const select = document.querySelector(selector);
      select.value = value;
      select.dispatchEvent(new Event("change", { bubbles: true }));
    };
    selectSource("#sourceASelect", "ocean_rocks");
    selectSource("#sourceBSelect", "drava_river_rapids");
    return true;
  `);
  await poll(async () => {
    const value = await playbackState(port, sessionId);
    return value.path.includes(
      "ocean_rocks__drava_river_rapids/windowed_convolution/"
    ) && value.status?.startsWith("rendered ") ? value : undefined;
  }, 90_000);

  const shortWindowStart = await playbackState(port, sessionId);
  const shortWindowTransition = await execute(port, sessionId, `
    for (const label of ["A window", "B window"]) {
      const input = document.querySelector(\`input[aria-label="\${label} exact value"]\`);
      input.value = "0.25";
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }
    return transportSnapshot();
  `);
  assert.equal(shortWindowTransition.paused, true, "stale audio must stop while rendering");
  assert.equal(shortWindowTransition.readyState, 0, "transport locks while rendering");
  assert.equal(
    shortWindowTransition.desiredPlaying,
    true,
    "render transition retains playing intent",
  );
  assert.ok(
    Math.abs(shortWindowTransition.currentTime - shortWindowStart.currentTime) < 0.2,
    "render transition freezes the audible sample clock",
  );
  const shortWindow = await poll(async () => {
    const value = await execute(port, sessionId, `
      const transport = transportSnapshot();
      return {
        duration: transport.duration,
        path: transport.path,
        paused: transport.paused,
        currentTime: transport.currentTime,
        status: document.querySelector("#renderStatus")?.textContent,
        error: document.querySelector("#errorPanel")?.textContent,
        errorHidden: document.querySelector("#errorPanel")?.hidden
      };
    `);
    return value.path.includes("/windowed_convolution/0.25x0.25/") &&
      value.status?.startsWith("rendered ") ? value : undefined;
  }, 90_000);
  assert.ok(
    Math.abs(shortWindow.duration - 122.5) < 0.02,
    `short-window convolution duration was ${shortWindow.duration}`,
  );
  assert.equal(shortWindow.errorHidden, true, shortWindow.error);
  assert.equal(shortWindow.paused, false, "new buffer resumes only after it is installed");
  assert.ok(
    shortWindow.currentTime >= shortWindowStart.currentTime - 0.2,
    "new duration preserves absolute time rather than normalized phase",
  );
  await captureMonitor();
  const shortWindowWav = await readFile(capturePath);
  const shortWindowSignal = analyzePcm16(shortWindowWav);
  const shortWindowRipple = phaseModulationDb(shortWindowWav, 6_000);
  assert.ok(shortWindowSignal.rms > 0.003, "short-window convolution is silent");
  assert.ok(
    shortWindowRipple < 2.5,
    `0.25-second windowed pulse ripple was ${shortWindowRipple.toFixed(2)} dB`,
  );

  await execute(port, sessionId, `
    const a = document.querySelector('input[aria-label="A window exact value"]');
    const b = document.querySelector('input[aria-label="B window exact value"]');
    a.value = "0.1";
    a.dispatchEvent(new Event("input", { bubbles: true }));
    b.value = "5";
    b.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  `);
  const unequalWindow = await poll(async () => {
    const value = await execute(port, sessionId, `
      const transport = transportSnapshot();
      return {
        duration: transport.duration,
        path: transport.path,
        status: document.querySelector("#renderStatus")?.textContent,
        error: document.querySelector("#errorPanel")?.textContent,
        errorHidden: document.querySelector("#errorPanel")?.hidden
      };
    `);
    return value.path.includes("/windowed_convolution/0.10x5.00/") &&
      value.status?.startsWith("rendered ") ? value : undefined;
  }, 180_000);
  assert.ok(
    Math.abs(unequalWindow.duration - 127.1) < 0.02,
    `0.1x5-second windowed duration was ${unequalWindow.duration}`,
  );
  assert.equal(unequalWindow.errorHidden, true, unequalWindow.error);
  await captureMonitor();
  const unequalWindowWav = await readFile(capturePath);
  const unequalWindowSignal = analyzePcm16(unequalWindowWav);
  const unequalWindowRipple = phaseModulationDb(unequalWindowWav, 2_400);
  const unequalRenderMs = Number(unequalWindow.status.match(/\d+/)?.[0]);
  assert.ok(Number.isFinite(unequalRenderMs), unequalWindow.status);
  assert.ok(unequalWindowSignal.rms > 0.003, "0.1x5 convolution is silent");
  assert.ok(
    unequalWindowRipple < 2.5,
    `0.1x5 windowed pulse ripple was ${unequalWindowRipple.toFixed(2)} dB`,
  );

  await execute(port, sessionId, `
    document.querySelector("#algorithmButtons button[data-value='chunk_crossfade']").click();
    const setCrossfade = () => {
      const input = document.querySelector("input[aria-label='overlap exact value']");
      if (!input) return false;
      input.value = "40";
      input.dispatchEvent(new Event("input", { bubbles: true }));
      return true;
    };
    return setCrossfade();
  `);
  const chunked = await poll(async () => {
    const value = await execute(port, sessionId, `
      const transport = transportSnapshot();
      return {
        duration: transport.duration,
        bufferRevision: transport.bufferRevision,
        path: transport.path,
        status: document.querySelector("#renderStatus")?.textContent,
        error: document.querySelector("#errorPanel")?.textContent,
        errorHidden: document.querySelector("#errorPanel")?.hidden
      };
    `);
    return value.path.includes("/chunk_crossfade/") &&
      value.status?.startsWith("rendered ") ? value : undefined;
  }, 180_000);
  assert.ok(
    Math.abs(chunked.duration - 65.5) < 0.02,
    `chunk duration was ${chunked.duration}`,
  );
  assert.equal(chunked.errorHidden, true, chunked.error);

  for (const [algorithm, expectedDuration, expectedToolInputs] of [
    ["source_filter_vocoder", 61, 6],
    ["moving_impulse_response", 61.75, 6],
  ]) {
    await execute(
      port,
      sessionId,
      `document.querySelector(
        "#algorithmButtons button[data-value='${algorithm}']"
      ).click(); return true;`,
    );
    const rendered = await poll(async () => {
      const value = await execute(port, sessionId, `
        const transport = transportSnapshot();
        return {
          duration: transport.duration,
          path: transport.path,
          toolInputs: document.querySelectorAll("#methodTools input").length,
          status: document.querySelector("#renderStatus")?.textContent,
          error: document.querySelector("#errorPanel")?.textContent,
          errorHidden: document.querySelector("#errorPanel")?.hidden
        };
      `);
      if (
        value.path.includes(`/${algorithm}/`) &&
        value.status?.startsWith("rendered ")
      ) {
        return value;
      }
      throw new Error(`waiting for ${algorithm}: ${JSON.stringify(value)}`);
    }, 90_000);
    assert.ok(
      Math.abs(rendered.duration - expectedDuration) < 0.02,
      `${algorithm} duration was ${rendered.duration}`,
    );
    assert.equal(rendered.toolInputs, expectedToolInputs);
    assert.equal(rendered.errorHidden, true, rendered.error);
  }

  await execute(
    port,
    sessionId,
    `document.querySelector("#algorithmButtons button[data-value='full_convolution']").click();
     return true;`,
  );
  const full = await poll(async () => {
    const value = await execute(port, sessionId, `
      const transport = transportSnapshot();
      return {
        duration: transport.duration,
        paused: transport.paused,
        bufferRevision: transport.bufferRevision,
        path: transport.path,
        status: document.querySelector("#renderStatus")?.textContent,
        toolInputs: document.querySelectorAll("#methodTools input").length,
        error: document.querySelector("#errorPanel")?.textContent,
        errorHidden: document.querySelector("#errorPanel")?.hidden
      };
    `);
    return value.path.endsWith(
      "/full_convolution/a0.00+61.00_b0.00+61.00",
    ) &&
      value.status?.startsWith("rendered ") ? value : undefined;
  }, 180_000);
  assert.ok(
    Math.abs(full.duration - 122) < 0.02,
    `default full-convolution duration was ${full.duration}`,
  );
  assert.equal(full.paused, false);
  assert.equal(full.toolInputs, 8);
  assert.equal(full.errorHidden, true, full.error);
  assert.ok(
    full.bufferRevision > initial.bufferRevision,
    "full convolution received a newly decoded AudioBuffer",
  );

  await execute(port, sessionId, `
    const setValue = (label, value) => {
      const input = document.querySelector(\`input[aria-label="\${label} exact value"]\`);
      input.value = String(value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    };
    setValue("A offset", 10);
    setValue("A duration", 20);
    setValue("B duration", 15);
    return true;
  `);
  const segmentedFull = await poll(async () => {
    const value = await execute(port, sessionId, `
      const transport = transportSnapshot();
      return {
        duration: transport.duration,
        path: transport.path,
        status: document.querySelector("#renderStatus")?.textContent,
        error: document.querySelector("#errorPanel")?.textContent,
        errorHidden: document.querySelector("#errorPanel")?.hidden
      };
    `);
    return value.path.endsWith(
      "/full_convolution/a10.00+20.00_b0.00+15.00",
    ) && value.status?.startsWith("rendered ") ? value : undefined;
  }, 180_000);
  assert.ok(
    Math.abs(segmentedFull.duration - 35) < 0.02,
    `segmented full-convolution duration was ${segmentedFull.duration}`,
  );
  assert.equal(segmentedFull.errorHidden, true, segmentedFull.error);

  await execute(port, sessionId, `
    const setValue = (label, value) => {
      const input = document.querySelector(\`input[aria-label="\${label} exact value"]\`);
      input.value = String(value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    };
    setValue("A duration", 0.5);
    setValue("B duration", 0.5);
    return true;
  `);
  const shortLoop = await poll(async () => {
    const value = await playbackState(port, sessionId);
    return value.path.endsWith(
      "/full_convolution/a10.00+0.50_b0.00+0.50",
    ) && value.readyState >= 4 ? value : undefined;
  }, 90_000);
  assert.ok(
    Math.abs(shortLoop.duration - 1) < 0.02,
    `short loop duration was ${shortLoop.duration}`,
  );
  const loopClockSamples = [];
  for (let index = 0; index < 24; index++) {
    loopClockSamples.push(await execute(port, sessionId, `
      return {
        ...transportSnapshot(),
        label: document.querySelector("#currentTime").textContent,
        seek: Number(document.querySelector("#seek").value)
      };
    `));
    await delay(70);
  }
  let observedWrap = false;
  for (let index = 0; index < loopClockSamples.length; index++) {
    const sample = loopClockSamples[index];
    assert.ok(
      circularTimeDistance(
        parseTimeLabel(sample.label),
        sample.currentTime,
        sample.duration,
      ) < 0.08,
      `time label diverged from played sample clock: ${JSON.stringify(sample)}`,
    );
    assert.ok(
      circularTimeDistance(sample.seek, sample.currentTime, sample.duration) < 0.08,
      `seek indicator diverged from played sample clock: ${JSON.stringify(sample)}`,
    );
    if (
      index > 0 &&
      loopClockSamples[index - 1].currentTime - sample.currentTime > 0.5
    ) {
      observedWrap = true;
    }
  }
  assert.equal(
    observedWrap,
    true,
    `native sample-clock playback did not visibly loop: ${JSON.stringify(loopClockSamples)}`,
  );

  await captureMonitor();
  const fullWav = await readFile(capturePath);
  const fullSignal = analyzePcm16(fullWav);
  const audibleLoopCorrelation = periodCorrelation(
    fullWav,
    Math.round(shortLoop.duration * 48_000),
  );
  assert.ok(fullSignal.rms > 0.003, `full convolution is silent: RMS ${fullSignal.rms}`);
  assert.ok(fullSignal.peak > 0.01, `full convolution peak is too low: ${fullSignal.peak}`);
  assert.ok(
    audibleLoopCorrelation > 0.98,
    `speaker output did not repeat the decoded one-second loop cleanly: ${audibleLoopCorrelation}`,
  );

  for (const algorithm of ["dry_a", "dry_b"]) {
    await execute(
      port,
      sessionId,
      `document.querySelector(
        "#algorithmButtons button[data-value='${algorithm}']"
      ).click(); return true;`,
    );
    const dry = await poll(async () => {
      const value = await execute(port, sessionId, `
        const transport = transportSnapshot();
        return {
          duration: transport.duration,
          path: transport.path,
          toolInputs: document.querySelectorAll("#methodTools input").length,
          tools: document.querySelector("#methodTools")?.textContent,
          status: document.querySelector("#renderStatus")?.textContent,
          error: document.querySelector("#errorPanel")?.textContent,
          errorHidden: document.querySelector("#errorPanel")?.hidden
        };
      `);
      return value.path.includes(`/${algorithm}/source`) &&
        value.status?.startsWith("rendered ") ? value : undefined;
    }, 90_000);
    assert.ok(Math.abs(dry.duration - 61) < 0.02, `${algorithm} duration was ${dry.duration}`);
    assert.equal(dry.toolInputs, 0);
    assert.match(dry.tools, /no configurable parameters/);
    assert.equal(dry.errorHidden, true, dry.error);
  }
  await captureMonitor();
  const drySignal = analyzePcm16(await readFile(capturePath));
  assert.ok(drySignal.rms > 0.003, `dry source is silent: RMS ${drySignal.rms}`);
  assert.ok(drySignal.peak > 0.01, `dry source peak is too low: ${drySignal.peak}`);

  const previousEpoch = await execute(port, sessionId, "return state.renderEpoch;");
  await execute(port, sessionId, "location.reload(); return true;");
  const reloaded = await poll(async () => {
    const value = await execute(port, sessionId, `
      if (typeof transportSnapshot !== "function") return null;
      const transport = transportSnapshot();
      return {
        ...transport,
        renderEpoch: state.renderEpoch,
        status: document.querySelector("#renderStatus")?.textContent,
        error: document.querySelector("#errorPanel")?.textContent,
        errorHidden: document.querySelector("#errorPanel")?.hidden
      };
    `);
    return value?.readyState >= 4 &&
      value.status?.startsWith("rendered ") ? value : undefined;
  }, 180_000);
  assert.ok(
    reloaded.renderEpoch > previousEpoch,
    `WebView reload reused render epoch ${reloaded.renderEpoch}`,
  );
  assert.match(reloaded.path, /windowed_convolution\/5\.00x5\.00\//);
  assert.equal(reloaded.paused, true, "reload must not retain stale playback intent");
  assert.equal(reloaded.errorHidden, true, reloaded.error);

  assert.equal(existsSync(retiredOutputDir), false, "native renders must not create outputs");
  console.log(
    `conv9 native audio passed: windowed ${decibels(signal.rms).toFixed(2)} dBFS RMS, ` +
      `0.25s ripple ${shortWindowRipple.toFixed(2)} dB, ` +
      `0.1x5s ripple ${unequalWindowRipple.toFixed(2)} dB in ${unequalRenderMs} ms, ` +
      `full ${decibels(fullSignal.rms).toFixed(2)} dBFS RMS`,
  );
} finally {
  if (sessionId && driver) {
    const port = driver.spawnargs.indexOf("--port");
    const driverPort = Number(driver.spawnargs[port + 1]);
    await webdriver(driverPort, "DELETE", `/session/${sessionId}`).catch(() => {});
  }
  if (driver && driver.exitCode === null) {
    driver.kill("SIGTERM");
    await waitForExit(driver, 2_000).catch(() => driver.kill("SIGKILL"));
  }
  if (moduleId) {
    spawnSync("pactl", ["unload-module", moduleId], { stdio: "ignore" });
  }
  for (const path of [capturePath, firstStartCapturePath, replayStartCapturePath]) {
    await unlink(path).catch(() => {});
  }
}

function loadNullSink() {
  const result = spawnSync(
    "pactl",
    [
      "load-module",
      "module-null-sink",
      `sink_name=${sinkName}`,
      "sink_properties=device.description=conv9-native-audio-test",
      "rate=48000",
      "channels=2",
    ],
    { encoding: "utf8" },
  );
  assert.equal(result.status, 0, result.stderr || "could not create PulseAudio test sink");
  const id = result.stdout.trim();
  assert.match(id, /^\d+$/);
  return id;
}

function nativeEnvironment() {
  const libraryDir = resolve(sysroot, "usr/lib64");
  const environment = {
    ...process.env,
    CONV9_MANIFEST: manifest,
    CONV9_INPUT_DIR: inputDir,
    PULSE_SINK: sinkName,
  };
  if (existsSync(libraryDir)) {
    environment.LD_LIBRARY_PATH = [libraryDir, process.env.LD_LIBRARY_PATH]
      .filter(Boolean)
      .join(":");
  }
  return environment;
}

async function playbackState(port, id) {
  return execute(port, id, `
    const transport = transportSnapshot();
    return {
      paused: transport.paused,
      currentTime: transport.currentTime,
      duration: transport.duration,
      readyState: transport.readyState,
      path: transport.path,
      contextState: transport.contextState,
      status: document.querySelector("#renderStatus")?.textContent,
      error: document.querySelector("#errorPanel")?.textContent,
      errorHidden: document.querySelector("#errorPanel")?.hidden
    };
  `);
}

async function execute(port, id, script) {
  return webdriver(port, "POST", `/session/${id}/execute/sync`, {
    script,
    args: [],
  });
}

async function executeAsync(port, id, script) {
  return webdriver(port, "POST", `/session/${id}/execute/async`, {
    script,
    args: [],
  });
}

async function webdriver(port, method, path, body) {
  const response = await fetch(`http://127.0.0.1:${port}${path}`, {
    method,
    headers: body ? { "Content-Type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const payload = await response.json();
  if (!response.ok || payload.value?.error) {
    throw new Error(
      `WebDriver ${method} ${path}: ${JSON.stringify(payload)}\n${driverOutput.join("")}`,
    );
  }
  return payload.value;
}

async function captureMonitor() {
  const capture = spawn(
    "ffmpeg",
    [
      "-hide_banner",
      "-loglevel",
      "error",
      "-f",
      "pulse",
      "-i",
      `${sinkName}.monitor`,
      "-t",
      "3",
      "-c:a",
      "pcm_s16le",
      "-y",
      capturePath,
    ],
    { stdio: ["ignore", "ignore", "pipe"] },
  );
  let errors = "";
  capture.stderr.on("data", (chunk) => {
    errors += chunk.toString();
  });
  const code = await waitForExit(capture, 10_000);
  assert.equal(code, 0, errors || `ffmpeg exited with ${code}`);
}

async function capturePlaybackStart(port, id, path) {
  const capture = spawn(
    "ffmpeg",
    [
      "-hide_banner",
      "-loglevel",
      "error",
      "-f",
      "pulse",
      "-i",
      `${sinkName}.monitor`,
      "-t",
      "3.2",
      "-c:a",
      "pcm_s16le",
      "-y",
      path,
    ],
    { stdio: ["ignore", "ignore", "pipe"] },
  );
  let errors = "";
  capture.stderr.on("data", (chunk) => {
    errors += chunk.toString();
  });
  await delay(500);
  await execute(port, id, `
    document.querySelector("#playButton").click();
    return true;
  `);
  await delay(2_200);
  await execute(port, id, `
    state.transport.desiredPlaying = false;
    stopTransport(true);
    return true;
  `);
  const code = await waitForExit(capture, 5_000);
  assert.equal(code, 0, errors || `first-play capture exited with ${code}`);
}

async function resetPlayback(port, id) {
  await execute(port, id, `
    state.transport.desiredPlaying = false;
    stopTransport(true);
    seekTransport(0);
    return true;
  `);
  await poll(async () => {
    const state = await playbackState(port, id);
    return state.paused && state.readyState >= 4 && state.currentTime < 0.005
      ? state
      : undefined;
  });
}

function compareCapturedToReference(
  capturedWav,
  referenceValues,
  decimation,
  referenceSampleRate,
) {
  const captured = monoPcm16(capturedWav);
  const reference = Float32Array.from(referenceValues);
  const captureSampleRate = 48_000;
  const reducedRate = referenceSampleRate / decimation;
  const searchStart = Math.floor(0.35 * captureSampleRate);
  const searchEnd = Math.floor(0.85 * captureSampleRate);
  const referenceStart = Math.floor(0.75 * reducedRate);
  const referenceEnd = Math.floor(1.8 * reducedRate);
  let lagFrames = 0;
  let best = -Infinity;
  for (let captureLag = searchStart; captureLag <= searchEnd; captureLag += 24) {
    const correlation = decimatedCorrelation(
      reference,
      captured,
      referenceStart,
      referenceEnd,
      captureLag,
      decimation,
      referenceSampleRate,
      captureSampleRate,
    );
    if (correlation > best) {
      best = correlation;
      lagFrames = captureLag;
    }
  }
  const coarseLag = lagFrames;
  for (let captureLag = coarseLag - 24; captureLag <= coarseLag + 24; captureLag++) {
    const correlation = decimatedCorrelation(
      reference,
      captured,
      referenceStart,
      referenceEnd,
      captureLag,
      decimation,
      referenceSampleRate,
      captureSampleRate,
    );
    if (correlation > best) {
      best = correlation;
      lagFrames = captureLag;
    }
  }
  const early = bestCorrelationNear(
    reference,
    captured,
    Math.floor(0.50 * reducedRate),
    Math.floor(1.05 * reducedRate),
    lagFrames,
    240,
    decimation,
    referenceSampleRate,
    captureSampleRate,
  );
  const late = bestCorrelationNear(
    reference,
    captured,
    Math.floor(1.10 * reducedRate),
    Math.floor(1.85 * reducedRate),
    lagFrames,
    240,
    decimation,
    referenceSampleRate,
    captureSampleRate,
  );
  return {
    lagFrames,
    earlyLagFrames: early.lagFrames,
    lateLagFrames: late.lagFrames,
    earlyCorrelation: early.correlation,
    lateCorrelation: late.correlation,
    prefixRmsDb: alignedRmsDb(captured, lagFrames, captureSampleRate, 0.1, 5),
  };
}

function alignedRmsDb(samples, startFrame, sampleRate, windowSeconds, count) {
  const frames = Math.round(sampleRate * windowSeconds);
  const levels = [];
  for (let window = 0; window < count; window++) {
    let power = 0;
    const start = startFrame + window * frames;
    const end = Math.min(samples.length, start + frames);
    for (let index = Math.max(0, start); index < end; index++) {
      power += samples[index] * samples[index];
    }
    levels.push(
      10 * Math.log10(Math.max(Number.EPSILON, power / Math.max(1, end - start))),
    );
  }
  return levels;
}

function bestCorrelationNear(
  reference,
  captured,
  start,
  end,
  centerLag,
  radius,
  decimation,
  referenceSampleRate,
  captureSampleRate,
) {
  let result = { lagFrames: centerLag, correlation: -Infinity };
  for (let lagFrames = centerLag - radius; lagFrames <= centerLag + radius; lagFrames++) {
    const correlation = decimatedCorrelation(
      reference,
      captured,
      start,
      end,
      lagFrames,
      decimation,
      referenceSampleRate,
      captureSampleRate,
    );
    if (correlation > result.correlation) result = { lagFrames, correlation };
  }
  return result;
}

function monoPcm16(wav) {
  const data = pcm16Data(wav);
  const channels = 2;
  const frames = Math.floor(data.length / (2 * channels));
  const samples = new Float32Array(frames);
  for (let frame = 0; frame < frames; frame++) {
    samples[frame] =
      (
        data.readInt16LE(frame * channels * 2) +
        data.readInt16LE((frame * channels + 1) * 2)
      ) /
      65536;
  }
  return samples;
}

function decimatedCorrelation(
  reference,
  captured,
  start,
  end,
  captureLag,
  decimation,
  referenceSampleRate,
  captureSampleRate,
) {
  let dot = 0;
  let referencePower = 0;
  let capturedPower = 0;
  for (let index = start; index < end; index++) {
    const referenceFrame = index * decimation;
    const capturedIndex =
      captureLag +
      Math.round((referenceFrame * captureSampleRate) / referenceSampleRate);
    if (
      index < 0 ||
      index >= reference.length ||
      capturedIndex < 0 ||
      capturedIndex >= captured.length
    ) {
      continue;
    }
    const expected = reference[index];
    const actual = captured[capturedIndex];
    dot += expected * actual;
    referencePower += expected * expected;
    capturedPower += actual * actual;
  }
  return dot / Math.sqrt(Math.max(Number.EPSILON, referencePower * capturedPower));
}

function analyzePcm16(wav) {
  const data = pcm16Data(wav);
  let sumSquares = 0;
  let peak = 0;
  const samples = Math.floor(data.length / 2);
  for (let index = 0; index < samples; index++) {
    const sample = data.readInt16LE(index * 2) / 32768;
    sumSquares += sample * sample;
    peak = Math.max(peak, Math.abs(sample));
  }
  return {
    samples,
    rms: Math.sqrt(sumSquares / Math.max(1, samples)),
    peak,
  };
}

function phaseModulationDb(wav, periodFrames) {
  const data = pcm16Data(wav);
  const channels = 2;
  const bins = 32;
  const power = new Float64Array(bins);
  const count = new Uint32Array(bins);
  const frames = Math.floor(data.length / (2 * channels));
  for (let frame = 0; frame < frames; frame++) {
    const bin = Math.floor(((frame % periodFrames) * bins) / periodFrames);
    for (let channel = 0; channel < channels; channel++) {
      const sample = data.readInt16LE((frame * channels + channel) * 2) / 32768;
      power[bin] += sample * sample;
      count[bin] += 1;
    }
  }
  const levels = [...power].map(
    (sum, index) => 10 * Math.log10(Math.max(sum / count[index], Number.EPSILON)),
  );
  return Math.max(...levels) - Math.min(...levels);
}

function periodCorrelation(wav, periodFrames) {
  const samples = monoPcm16(wav);
  const start = Math.floor(0.25 * 48_000);
  const end = Math.min(
    samples.length - periodFrames,
    Math.floor(2.5 * 48_000),
  );
  let dot = 0;
  let leftPower = 0;
  let rightPower = 0;
  for (let index = start; index < end; index++) {
    const left = samples[index];
    const right = samples[index + periodFrames];
    dot += left * right;
    leftPower += left * left;
    rightPower += right * right;
  }
  return dot / Math.sqrt(Math.max(Number.EPSILON, leftPower * rightPower));
}

function pcm16Data(wav) {
  let offset = 12;
  let data;
  while (offset + 8 <= wav.length) {
    const id = wav.toString("ascii", offset, offset + 4);
    const length = wav.readUInt32LE(offset + 4);
    if (id === "data") {
      data = wav.subarray(offset + 8, offset + 8 + length);
      break;
    }
    offset += 8 + length + (length % 2);
  }
  assert.ok(data, "capture has no WAV data chunk");
  return data;
}

function decibels(amplitude) {
  return 20 * Math.log10(Math.max(amplitude, Number.EPSILON));
}

function parseTimeLabel(label) {
  const [minutes, seconds] = label.split(":");
  return Number(minutes) * 60 + Number(seconds);
}

function circularTimeDistance(left, right, duration) {
  const distance = Math.abs(left - right);
  return Math.min(distance, Math.max(0, duration - distance));
}

function command(binary, arguments_) {
  const result = spawnSync(binary, arguments_, { encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr || `${binary} failed`);
  return result.stdout;
}

function delay(milliseconds) {
  return new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds));
}

async function poll(check, timeout = 30_000) {
  const deadline = Date.now() + timeout;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const result = await check();
      if (result !== undefined) return result;
    } catch (error) {
      // WebKitWebDriver can transiently reject script-result serialization
      // while the webview's synchronous spectrum pass owns the UI thread.
      lastError = error;
    }
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 100));
  }
  throw new Error(
    `timed out waiting for native app state${lastError ? `: ${lastError.message}` : ""}`,
  );
}

async function availablePort() {
  const server = createServer();
  await new Promise((resolveListen, rejectListen) => {
    server.once("error", rejectListen);
    server.listen(0, "127.0.0.1", resolveListen);
  });
  const address = server.address();
  assert.ok(address && typeof address === "object");
  await new Promise((resolveClose) => server.close(resolveClose));
  return address.port;
}

async function waitForPort(port, child) {
  await poll(
    () =>
      new Promise((resolveConnection, rejectConnection) => {
        if (child.exitCode !== null) {
          rejectConnection(
            new Error(`tauri-driver exited with ${child.exitCode}\n${driverOutput.join("")}`),
          );
          return;
        }
        const socket = createConnection({ host: "127.0.0.1", port });
        socket.once("connect", () => {
          socket.destroy();
          resolveConnection(true);
        });
        socket.once("error", () => resolveConnection(undefined));
      }),
  );
}

function waitForExit(child, timeout) {
  if (child.exitCode !== null) return Promise.resolve(child.exitCode);
  return new Promise((resolveExit, rejectExit) => {
    const timer = setTimeout(() => rejectExit(new Error("process exit timed out")), timeout);
    child.once("exit", (code) => {
      clearTimeout(timer);
      resolveExit(code);
    });
  });
}
