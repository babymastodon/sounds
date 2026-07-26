import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { readFile, unlink } from "node:fs/promises";
import { createConnection, createServer } from "node:net";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const appDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const conv9Dir = resolve(appDir, "..");
const application = resolve(appDir, "src-tauri/target/debug/conv9-listener");
const manifest = resolve(conv9Dir, "sources.tsv");
const inputDir = resolve(conv9Dir, "samples/prepared");
const retiredOutputDir = resolve(conv9Dir, "outputs");
const driverBinary =
  process.env.TAURI_DRIVER_BIN || "/tmp/conv9-tauri-driver/bin/tauri-driver";
const sysroot = process.env.CONV9_TAURI_SYSROOT || "/tmp/conv9-tauri-devel";
const sinkName = `conv9_test_${process.pid}`;
const capturePath = `/tmp/${sinkName}.wav`;

assert.ok(existsSync(application), `build the Tauri app first: missing ${application}`);
assert.ok(
  existsSync(driverBinary),
  `install tauri-driver or set TAURI_DRIVER_BIN: missing ${driverBinary}`,
);
assert.ok(existsSync(manifest), "conv9 source manifest is missing");
assert.ok(existsSync(resolve(inputDir, "ambient_guitar.wav")), "prepared inputs are missing");
assert.equal(existsSync(retiredOutputDir), false, "precomputed output tree must remain absent");

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
      const audio = document.querySelector("#audio");
      const waveform = document.querySelector("#waveform");
      const spectrum = document.querySelector("#spectrogram");
      return {
        readyState: audio?.readyState,
        duration: audio?.duration,
        source: audio?.currentSrc,
        path: audio?.dataset.path,
        loop: audio?.loop,
        playbackRate: audio?.playbackRate,
        playbackSpeedValue: document.querySelector("#playbackSpeedValue")?.textContent,
        title: document.querySelector("#renderTitle")?.textContent,
        status: document.querySelector("#renderStatus")?.textContent,
        sourceACount: document.querySelector("#sourceASelect")?.options.length,
        sourceBCount: document.querySelector("#sourceBSelect")?.options.length,
        methodCount: document.querySelectorAll("#algorithmButtons button").length,
        windowCount: document.querySelectorAll("#methodTools .window-control").length,
        playDisabled: document.querySelector("#playButton")?.disabled,
        seekDisabled: document.querySelector("#seek")?.disabled,
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
  assert.match(initial.source, /^blob:tauri:/);
  assert.match(initial.path, /windowed_convolution\/5\.00x5\.00\//);
  assert.equal(initial.loop, true);
  assert.equal(initial.playbackRate, 1);
  assert.equal(initial.playbackSpeedValue, "1.00×");
  assert.equal(initial.title, "windowed");
  assert.match(initial.status, /^rendered \d+ ms$/);
  assert.equal(initial.sourceACount, 48);
  assert.equal(initial.sourceBCount, 48);
  assert.equal(initial.methodCount, 6);
  assert.equal(initial.windowCount, 2);
  assert.equal(initial.playDisabled, false);
  assert.equal(initial.seekDisabled, false);
  assert.deepEqual(initial.missingTooltips, []);
  assert.equal(initial.fftSize, "16384");
  assert.equal(initial.errorHidden, true, initial.error);
  assert.ok(initial.viewport.scrollWidth <= initial.viewport.width, "native horizontal overflow");
  assert.ok(initial.viewport.scrollHeight <= initial.viewport.height, "native vertical overflow");

  const changedPlaybackRate = await execute(port, sessionId, `
    const speed = document.querySelector("#playbackSpeed");
    speed.value = "1.35";
    speed.dispatchEvent(new Event("input", { bubbles: true }));
    const result = {
      rate: document.querySelector("#audio").playbackRate,
      value: document.querySelector("#playbackSpeedValue").textContent
    };
    speed.value = "1";
    speed.dispatchEvent(new Event("input", { bubbles: true }));
    return result;
  `);
  assert.equal(changedPlaybackRate.rate, 1.35);
  assert.equal(changedPlaybackRate.value, "1.35×");

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
    for (const label of ["A window", "B window"]) {
      const input = document.querySelector(\`input[aria-label="\${label} exact value"]\`);
      input.value = "0.25";
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }
    return true;
  `);
  const shortWindow = await poll(async () => {
    const value = await execute(port, sessionId, `
      const audio = document.querySelector("#audio");
      return {
        duration: audio.duration,
        path: audio.dataset.path,
        status: document.querySelector("#renderStatus")?.textContent,
        readout: document.querySelector("#windowReadout")?.textContent,
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
  assert.match(shortWindow.readout, /overlap 75%/);
  assert.equal(shortWindow.errorHidden, true, shortWindow.error);
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
      const audio = document.querySelector("#audio");
      return {
        duration: audio.duration,
        path: audio.dataset.path,
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
      const audio = document.querySelector("#audio");
      return {
        duration: audio.duration,
        source: audio.currentSrc,
        path: audio.dataset.path,
        title: document.querySelector("#renderTitle")?.textContent,
        readout: document.querySelector("#windowReadout")?.textContent,
        status: document.querySelector("#renderStatus")?.textContent,
        error: document.querySelector("#errorPanel")?.textContent,
        errorHidden: document.querySelector("#errorPanel")?.hidden
      };
    `);
    return value.path.includes("/chunk_crossfade/") &&
      value.status?.startsWith("rendered ") ? value : undefined;
  }, 180_000);
  assert.equal(chunked.title, "chunks");
  assert.ok(
    Math.abs(chunked.duration - 65.5) < 0.02,
    `chunk duration was ${chunked.duration}`,
  );
  assert.match(chunked.readout, /40%/);
  assert.equal(chunked.errorHidden, true, chunked.error);

  for (const [algorithm, expectedDuration] of [["evolving_ir", 66]]) {
    await execute(
      port,
      sessionId,
      `document.querySelector(
        "#algorithmButtons button[data-value='${algorithm}']"
      ).click(); return true;`,
    );
    const rendered = await poll(async () => {
      const value = await execute(port, sessionId, `
        const audio = document.querySelector("#audio");
        return {
          duration: audio.duration,
          path: audio.dataset.path,
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
      const audio = document.querySelector("#audio");
      return {
        duration: audio.duration,
        paused: audio.paused,
        source: audio.currentSrc,
        path: audio.dataset.path,
        title: document.querySelector("#renderTitle")?.textContent,
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
  assert.equal(full.title, "full");
  assert.equal(full.toolInputs, 8);
  assert.equal(full.errorHidden, true, full.error);
  assert.notEqual(full.source, initial.source, "full convolution received a new in-memory WAV");

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
      const audio = document.querySelector("#audio");
      return {
        duration: audio.duration,
        path: audio.dataset.path,
        status: document.querySelector("#renderStatus")?.textContent,
        readout: document.querySelector("#windowReadout")?.textContent,
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
  assert.match(segmentedFull.readout, /out 35\.00s/);
  assert.equal(segmentedFull.errorHidden, true, segmentedFull.error);

  await captureMonitor();
  const fullSignal = analyzePcm16(await readFile(capturePath));
  assert.ok(fullSignal.rms > 0.003, `full convolution is silent: RMS ${fullSignal.rms}`);
  assert.ok(fullSignal.peak > 0.01, `full convolution peak is too low: ${fullSignal.peak}`);

  for (const [algorithm, title] of [
    ["dry_a", "dry a"],
    ["dry_b", "dry b"],
  ]) {
    await execute(
      port,
      sessionId,
      `document.querySelector(
        "#algorithmButtons button[data-value='${algorithm}']"
      ).click(); return true;`,
    );
    const dry = await poll(async () => {
      const value = await execute(port, sessionId, `
        const audio = document.querySelector("#audio");
        return {
          duration: audio.duration,
          path: audio.dataset.path,
          title: document.querySelector("#renderTitle")?.textContent,
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
    assert.equal(dry.title, title);
    assert.equal(dry.toolInputs, 0);
    assert.match(dry.tools, /no configurable parameters/);
    assert.equal(dry.errorHidden, true, dry.error);
  }
  await captureMonitor();
  const drySignal = analyzePcm16(await readFile(capturePath));
  assert.ok(drySignal.rms > 0.003, `dry source is silent: RMS ${drySignal.rms}`);
  assert.ok(drySignal.peak > 0.01, `dry source peak is too low: ${drySignal.peak}`);

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
  await unlink(capturePath).catch(() => {});
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
    const audio = document.querySelector("#audio");
    return {
      paused: audio.paused,
      currentTime: audio.currentTime,
      readyState: audio.readyState,
      errorCode: audio.error?.code,
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

function command(binary, arguments_) {
  const result = spawnSync(binary, arguments_, { encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr || `${binary} failed`);
  return result.stdout;
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
