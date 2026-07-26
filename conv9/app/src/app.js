const ui = {
  renderStatus: document.querySelector("#renderStatus"),
  sourceASelect: document.querySelector("#sourceASelect"),
  sourceBSelect: document.querySelector("#sourceBSelect"),
  sourceAMeta: document.querySelector("#sourceAMeta"),
  sourceBMeta: document.querySelector("#sourceBMeta"),
  algorithmButtons: document.querySelector("#algorithmButtons"),
  methodToolTitle: document.querySelector("#methodToolTitle"),
  methodTools: document.querySelector("#methodTools"),
  renderTitle: document.querySelector("#renderTitle"),
  metrics: document.querySelector("#metrics"),
  audio: document.querySelector("#audio"),
  playButton: document.querySelector("#playButton"),
  seek: document.querySelector("#seek"),
  volume: document.querySelector("#volume"),
  playbackSpeed: document.querySelector("#playbackSpeed"),
  playbackSpeedValue: document.querySelector("#playbackSpeedValue"),
  currentTime: document.querySelector("#currentTime"),
  duration: document.querySelector("#duration"),
  waveform: document.querySelector("#waveform"),
  spectrogram: document.querySelector("#spectrogram"),
  windowReadout: document.querySelector("#windowReadout"),
  errorPanel: document.querySelector("#errorPanel"),
};

const state = {
  catalog: null,
  bridge: null,
  sourceA: "",
  sourceB: "",
  algorithm: "windowed_convolution",
  settings: new Map(),
  waveformLayer: null,
  spectrumLayer: null,
  analysisSamples: null,
  analysisSampleRate: 0,
  audioObjectUrl: "",
  audioReady: false,
  selectionGeneration: 0,
  selectionTimer: 0,
};

async function boot() {
  try {
    state.bridge = runtimeBridge();
    const bootstrap = await state.bridge.loadBootstrap();
    state.catalog = bootstrap.catalog;
    if (state.catalog.mode !== "on_demand") {
      throw new Error("The backend did not provide an on-demand catalog.");
    }
    if (state.catalog.sources.length < 2) {
      throw new Error("At least two prepared clips are required.");
    }
    state.sourceA = state.catalog.sources[0].id;
    state.sourceB = state.catalog.sources[1].id;
    for (const algorithm of state.catalog.algorithms) {
      state.settings.set(algorithm.id, {
        windows: Object.fromEntries(
          algorithm.windows.map((window) => [window.id, window.default]),
        ),
        parameters: Object.fromEntries(
          algorithm.parameters.map((parameter) => [parameter.id, parameter.default]),
        ),
      });
    }
    buildControls();
    bindEvents();
    ui.renderStatus.textContent = "on demand";
    await selectClip(false, ++state.selectionGeneration);
    requestAnimationFrame(animateCursor);
  } catch (error) {
    showError(error);
  }
}

function runtimeBridge() {
  if (window.__CONV9_TEST_BRIDGE__) return window.__CONV9_TEST_BRIDGE__;
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) {
    throw new Error("The on-demand renderer requires the conv9 Tauri desktop app.");
  }
  return {
    loadBootstrap: () => invoke("load_bootstrap"),
    renderSelection: async (request) => parseEnvelope(await invoke("render_selection", request)),
    supersedeRender: (requestId) =>
      invoke("supersede_render", { requestId }).catch(() => {}),
  };
}

function buildControls() {
  for (const source of state.catalog.sources) {
    const option = document.createElement("option");
    option.value = source.id;
    option.textContent = `${source.category} / ${source.kind}`;
    option.title =
      `Use “${source.category}” as clip A. This ${source.kind} source is ` +
      `${source.seconds.toFixed(0)} seconds and will be conditioned before convolution.`;
    ui.sourceASelect.append(option);
    const optionB = option.cloneNode(true);
    optionB.title =
      `Use “${source.category}” as clip B. This ${source.kind} source is ` +
      `${source.seconds.toFixed(0)} seconds and will be conditioned before convolution.`;
    ui.sourceBSelect.append(optionB);
  }
  ui.sourceASelect.value = state.sourceA;
  ui.sourceBSelect.value = state.sourceB;

  for (const algorithm of state.catalog.algorithms) {
    const button = document.createElement("button");
    button.type = "button";
    button.dataset.value = algorithm.id;
    button.title =
      `${algorithm.title}. ${algorithm.description} Selecting it renders immediately ` +
      `and shows only this method’s applicable controls.`;
    button.textContent = shortAlgorithm(algorithm.id);
    ui.algorithmButtons.append(button);
  }
  refreshButtons();
  buildMethodTools();
  refreshSourceMetadata();
}

function bindEvents() {
  ui.sourceASelect.addEventListener("change", () => {
    state.sourceA = ui.sourceASelect.value;
    refreshSourceMetadata();
    scheduleSelection(true);
  });
  ui.sourceBSelect.addEventListener("change", () => {
    state.sourceB = ui.sourceBSelect.value;
    refreshSourceMetadata();
    scheduleSelection(true);
  });
  ui.algorithmButtons.addEventListener("click", (event) => {
    const button = event.target.closest("button");
    if (!button) return;
    state.algorithm = button.dataset.value;
    refreshButtons();
    buildMethodTools();
    scheduleSelection(true);
  });
  ui.playButton.addEventListener("click", () => {
    void togglePlayback().catch(showError);
  });
  ui.audio.addEventListener("play", refreshPlayButton);
  ui.audio.addEventListener("pause", refreshPlayButton);
  ui.audio.addEventListener("loadedmetadata", () => {
    applyPlaybackSpeed();
    refreshTransport();
  });
  ui.audio.addEventListener("timeupdate", refreshTransport);
  ui.audio.addEventListener("ended", refreshPlayButton);
  ui.seek.addEventListener("input", () => {
    ui.audio.currentTime = Number(ui.seek.value);
  });
  ui.volume.addEventListener("input", () => {
    ui.audio.volume = Number(ui.volume.value);
  });
  ui.playbackSpeed.addEventListener("input", applyPlaybackSpeed);
  ui.audio.volume = Number(ui.volume.value);
  applyPlaybackSpeed();
  for (const canvas of [ui.waveform, ui.spectrogram]) {
    canvas.addEventListener("click", (event) => {
      if (!state.audioReady) return;
      const bounds = canvas.getBoundingClientRect();
      const phase = Math.max(0, Math.min(1, (event.clientX - bounds.left) / bounds.width));
      ui.audio.currentTime = phase * (ui.audio.duration || state.catalog.input_seconds);
    });
  }
  window.addEventListener("resize", debounce(resizeVisualizations, 180));
  document.addEventListener("keydown", (event) => {
    if (event.target.matches("select, input")) return;
    if (event.code === "Space") {
      event.preventDefault();
      void togglePlayback().catch(showError);
    } else if (event.code === "ArrowLeft") {
      if (!state.audioReady) return;
      ui.audio.currentTime = Math.max(0, ui.audio.currentTime - 5);
    } else if (event.code === "ArrowRight") {
      if (!state.audioReady) return;
      ui.audio.currentTime = Math.min(
        ui.audio.duration || state.catalog.input_seconds,
        ui.audio.currentTime + 5,
      );
    }
  });
}

function applyPlaybackSpeed() {
  const rate = 2 ** Number(ui.playbackSpeed.value);
  ui.audio.defaultPlaybackRate = rate;
  ui.audio.playbackRate = rate;
  ui.audio.preservesPitch = true;
  ui.playbackSpeedValue.value = `${rate.toFixed(2)}×`;
  ui.playbackSpeed.setAttribute("aria-valuetext", `${rate.toFixed(2)} times`);
}

function buildMethodTools() {
  const algorithm = selectedAlgorithm();
  const settings = state.settings.get(algorithm.id);
  ui.methodToolTitle.textContent = `${shortAlgorithm(algorithm.id)} / tools`;
  ui.methodTools.replaceChildren();
  if (!algorithm.windows.length && !algorithm.parameters.length) {
    const summary = document.createElement("span");
    summary.className = "method-summary";
    summary.textContent = "no configurable parameters";
    ui.methodTools.append(summary);
    return;
  }
  for (const descriptor of algorithm.windows) {
    ui.methodTools.append(
      toolControl(descriptor, settings.windows, {
        logarithmic: descriptor.scale === "soft_log",
        window: true,
      }),
    );
  }
  for (const descriptor of algorithm.parameters) {
    ui.methodTools.append(toolControl(descriptor, settings.parameters));
  }
}

function toolControl(descriptor, target, options = {}) {
  const control = document.createElement("label");
  control.className = `tool-control${options.window ? " window-control" : ""}`;
  control.title = descriptor.description;
  const label = document.createElement("span");
  label.textContent = descriptor.label;
  const inputs = document.createElement("span");
  inputs.className = "tool-inputs";
  const slider = document.createElement("input");
  slider.type = "range";
  slider.dataset.controlId = descriptor.id;
  slider.ariaLabel = descriptor.label;
  slider.setAttribute("aria-description", descriptor.description);
  const scaleExplanation = options.logarithmic
    ? "Slider travel uses a softened logarithmic curve: sub-second values retain detail without crowding multi-second values."
    : "Slider travel is linear across the stated range.";
  slider.title =
    `${descriptor.description} ${scaleExplanation} Drag to adjust from ${descriptor.minimum} to ` +
    `${descriptor.maximum}${options.window ? " seconds" : descriptor.unit}. ` +
    `Changing it triggers a new on-demand render.`;
  const number = document.createElement("input");
  number.type = "number";
  number.dataset.controlId = descriptor.id;
  number.min = descriptor.minimum;
  number.max = descriptor.maximum;
  number.step = descriptor.step;
  number.ariaLabel = `${descriptor.label} exact value`;
  number.setAttribute("aria-description", descriptor.description);
  number.title =
    `${descriptor.description} Enter an exact value from ${descriptor.minimum} to ` +
    `${descriptor.maximum}${options.window ? " seconds" : descriptor.unit}; ` +
    `the adjacent slider follows it and the selection is rendered again.`;
  const unit = document.createElement("span");
  unit.className = "tool-unit";
  unit.textContent = options.window ? "s" : descriptor.unit;
  const initial = target[descriptor.id];
  if (options.logarithmic) {
    slider.min = "0";
    slider.max = "1000";
    slider.step = "1";
    slider.value = logPosition(initial, descriptor.minimum, descriptor.maximum);
  } else {
    slider.min = descriptor.minimum;
    slider.max = descriptor.maximum;
    slider.step = descriptor.step;
    slider.value = initial;
  }
  number.value = formatControlValue(initial, descriptor.step);
  slider.setAttribute("aria-valuetext", `${number.value}${unit.textContent}`);

  const applyValue = (value) => {
    if (!Number.isFinite(value)) {
      number.value = formatControlValue(target[descriptor.id], descriptor.step);
      return;
    }
    const safe = clamp(roundToStep(value, descriptor.step), descriptor.minimum, descriptor.maximum);
    target[descriptor.id] = safe;
    normalizeFullSegments(target, descriptor.id);
    number.value = formatControlValue(safe, descriptor.step);
    slider.value = options.logarithmic
      ? logPosition(safe, descriptor.minimum, descriptor.maximum)
      : safe;
    slider.setAttribute("aria-valuetext", `${number.value}${unit.textContent}`);
    syncCoupledControls(target, descriptor.id);
    scheduleSelection(true);
  };
  slider.addEventListener("input", () => {
    const value = options.logarithmic
      ? logValue(Number(slider.value), descriptor.minimum, descriptor.maximum)
      : Number(slider.value);
    applyValue(value);
  });
  number.addEventListener("input", () => {
    const value = Number(number.value);
    if (Number.isFinite(value) && value >= descriptor.minimum && value <= descriptor.maximum) {
      applyValue(value);
    }
  });
  number.addEventListener("change", () => applyValue(Number(number.value)));
  inputs.append(slider, number, unit);
  control.append(label, inputs);
  return control;
}

function scheduleSelection(preservePlayback) {
  const generation = ++state.selectionGeneration;
  clearTimeout(state.selectionTimer);
  state.bridge.supersedeRender?.(generation);
  ui.renderStatus.textContent = "queued";
  state.selectionTimer = setTimeout(() => {
    void selectClip(preservePlayback, generation);
  }, 140);
}

async function selectClip(preservePlayback, generation) {
  try {
    hideError();
    const oldDuration = Number.isFinite(ui.audio.duration)
      ? ui.audio.duration
      : state.catalog.input_seconds;
    const phase = preservePlayback ? ui.audio.currentTime / oldDuration : 0;
    const resume = preservePlayback && !ui.audio.paused;
    const algorithm = selectedAlgorithm();
    const settings = state.settings.get(algorithm.id);
    const request = {
      requestId: generation,
      leftId: state.sourceA,
      rightId: state.sourceB,
      algorithm: state.algorithm,
      windows: { ...settings.windows },
      parameters: { ...settings.parameters },
    };
    state.analysisSamples = null;
    state.analysisSampleRate = 0;
    state.waveformLayer = null;
    state.spectrumLayer = null;
    ui.renderStatus.textContent = "rendering…";
    drawLoading(ui.waveform, "RENDERING ON DEMAND");
    drawLoading(ui.spectrogram, "RENDERING ON DEMAND");
    const rendered = await state.bridge.renderSelection(request);
    if (generation !== state.selectionGeneration) return;
    const bytes = normalizeBytes(rendered.wav);
    const analysisBytes = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
    const objectUrl = URL.createObjectURL(new Blob([bytes], { type: "audio/wav" }));
    const previousObjectUrl = state.audioObjectUrl;
    state.audioObjectUrl = objectUrl;
    setTransportReady(false);
    ui.audio.pause();
    ui.audio.dataset.path = selectionSignature(request);
    const metadataLoaded = once(ui.audio, "loadedmetadata");
    ui.audio.src = objectUrl;
    ui.audio.loop = true;
    ui.audio.load();
    await metadataLoaded;
    if (previousObjectUrl) URL.revokeObjectURL(previousObjectUrl);
    if (generation !== state.selectionGeneration) return;
    ui.audio.currentTime = Math.max(
      0,
      Math.min(ui.audio.duration - 0.01, phase * ui.audio.duration),
    );
    updateMetadata(rendered.header, settings);
    ui.renderStatus.textContent = "analyzing…";
    await analyzeSelection(analysisBytes, generation);
    if (generation !== state.selectionGeneration) return;
    setTransportReady(true);
    ui.renderStatus.textContent = `rendered ${rendered.header.renderMilliseconds} ms`;
    if (resume) await ui.audio.play();
  } catch (error) {
    if (generation === state.selectionGeneration) {
      ui.renderStatus.textContent = "failed";
      showError(error);
    }
  }
}

function updateMetadata(header, settings) {
  ui.renderTitle.textContent = shortAlgorithm(state.algorithm);
  ui.metrics.innerHTML = [
    metricMarkup("rms", `${header.metrics.rms_dbfs.toFixed(1)} dbfs`),
    metricMarkup("peak", `${(header.metrics.peak * 100).toFixed(1)}%`),
  ].join("");
  const clipASeconds = header.windows.clip_a_seconds;
  const clipBSeconds = header.windows.clip_b_seconds;
  if (clipASeconds == null) {
    if (state.algorithm === "full_convolution") {
      const parameters = settings.parameters;
      ui.windowReadout.textContent =
        `a ${parameters.full_a_duration_seconds.toFixed(1)}s @ ` +
        `${parameters.full_a_offset_seconds.toFixed(1)}s / ` +
        `b ${parameters.full_b_duration_seconds.toFixed(1)}s @ ` +
        `${parameters.full_b_offset_seconds.toFixed(1)}s / ` +
        `out ${header.metrics.duration_seconds.toFixed(2)}s`;
    } else {
      const source = state.algorithm === "dry_a" ? "a" : "b";
      ui.windowReadout.textContent =
        `source ${source} / out ${header.metrics.duration_seconds.toFixed(2)}s`;
    }
  } else {
    let readout =
      `a ${clipASeconds.toFixed(2)}s / b ${clipBSeconds.toFixed(2)}s / ` +
      `scan ${header.hopSeconds.toFixed(2)}s`;
    if (state.algorithm === "chunk_crossfade") {
      const percentage = settings.parameters.chunk_crossfade_percent;
      const duration = Math.min(clipASeconds, clipBSeconds) * percentage / 100;
      readout += ` / overlap ${duration.toFixed(2)}s (${percentage.toFixed(0)}%)`;
    } else {
      readout += ` / overlap ${settings.parameters.window_overlap_percent.toFixed(0)}%`;
    }
    readout += ` / out ${header.metrics.duration_seconds.toFixed(2)}s`;
    ui.windowReadout.textContent = readout;
  }
}

function refreshSourceMetadata() {
  const sources = new Map(state.catalog.sources.map((source) => [source.id, source]));
  ui.sourceAMeta.innerHTML = sourceMetadata(sources.get(state.sourceA));
  ui.sourceBMeta.innerHTML = sourceMetadata(sources.get(state.sourceB));
}

function sourceMetadata(source) {
  return `${escapeHtml(source.creator)} / ${escapeHtml(source.license)} / ` +
    `<a href="${escapeHtml(source.source_page)}" ` +
    `title="Open the original source and license page for ${escapeHtml(source.category)}.">source</a>`;
}

function metricMarkup(label, value) {
  return `<div><dt>${label}</dt><dd>${value}</dd></div>`;
}

function selectedAlgorithm() {
  return state.catalog.algorithms.find((algorithm) => algorithm.id === state.algorithm);
}

function selectionSignature(request) {
  if (request.windows.clip_a_seconds == null) {
    if (request.algorithm !== "full_convolution") {
      return `${request.leftId}__${request.rightId}/${request.algorithm}/source`;
    }
    const segments = `a${request.parameters.full_a_offset_seconds.toFixed(2)}+` +
      `${request.parameters.full_a_duration_seconds.toFixed(2)}_` +
      `b${request.parameters.full_b_offset_seconds.toFixed(2)}+` +
      `${request.parameters.full_b_duration_seconds.toFixed(2)}`;
    return `${request.leftId}__${request.rightId}/${request.algorithm}/${segments}`;
  }
  const windows =
    `${request.windows.clip_a_seconds.toFixed(2)}x` +
    `${request.windows.clip_b_seconds.toFixed(2)}`;
  const parameters = Object.entries(request.parameters)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([id, value]) => `${id}=${Number(value).toFixed(2)}`)
    .join(",");
  return `${request.leftId}__${request.rightId}/${request.algorithm}/${windows}/${parameters}`;
}

function normalizeFullSegments(target, changedId) {
  if (!changedId.startsWith("full_")) return;
  const clip = changedId.startsWith("full_a_") ? "a" : "b";
  const offsetId = `full_${clip}_offset_seconds`;
  const durationId = `full_${clip}_duration_seconds`;
  const inputSeconds = state.catalog.input_seconds;
  if (changedId === offsetId && target[offsetId] + target[durationId] > inputSeconds) {
    target[durationId] = roundToStep(
      Math.max(0.1, inputSeconds - target[offsetId]),
      0.1,
    );
  } else if (
    changedId === durationId &&
    target[offsetId] + target[durationId] > inputSeconds
  ) {
    target[offsetId] = roundToStep(
      Math.max(0, inputSeconds - target[durationId]),
      0.1,
    );
  }
}

function syncCoupledControls(target, changedId) {
  if (!changedId.startsWith("full_")) return;
  const clip = changedId.startsWith("full_a_") ? "a" : "b";
  for (const id of [`full_${clip}_offset_seconds`, `full_${clip}_duration_seconds`]) {
    const value = target[id];
    const number = ui.methodTools.querySelector(
      `input[type="number"][data-control-id="${id}"]`,
    );
    const slider = ui.methodTools.querySelector(
      `input[type="range"][data-control-id="${id}"]`,
    );
    if (!number || !slider) continue;
    number.value = formatControlValue(value, Number(number.step));
    slider.value = value;
    slider.setAttribute("aria-valuetext", `${number.value}s`);
  }
}

function parseEnvelope(raw) {
  const bytes = normalizeBytes(raw);
  if (
    bytes.length < 12 ||
    String.fromCharCode(...bytes.subarray(0, 4)) !== "CV9R"
  ) {
    throw new Error("The renderer returned an invalid audio envelope.");
  }
  const headerLength = new DataView(
    bytes.buffer,
    bytes.byteOffset + 4,
    4,
  ).getUint32(0, true);
  const wavOffset = 8 + headerLength;
  if (wavOffset + 44 > bytes.length) {
    throw new Error("The renderer returned a truncated audio envelope.");
  }
  const header = JSON.parse(new TextDecoder().decode(bytes.subarray(8, wavOffset)));
  return { header, wav: bytes.subarray(wavOffset) };
}

function normalizeBytes(value) {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  if (Array.isArray(value)) return Uint8Array.from(value);
  throw new Error(`Unsupported binary render response: ${typeof value}`);
}

function logPosition(value, minimum, maximum) {
  const softness = 0.25;
  return Math.round(
    1000 *
      Math.log((value + softness) / (minimum + softness)) /
      Math.log((maximum + softness) / (minimum + softness)),
  );
}

function logValue(position, minimum, maximum) {
  const softness = 0.25;
  return (
    (minimum + softness) *
      Math.pow((maximum + softness) / (minimum + softness), position / 1000) -
    softness
  );
}

function roundToStep(value, step) {
  const decimals = String(step).split(".")[1]?.length ?? 0;
  return Number((Math.round(value / step) * step).toFixed(decimals));
}

function formatControlValue(value, step) {
  const decimals = String(step).split(".")[1]?.length ?? 0;
  return Number(value).toFixed(decimals);
}

function clamp(value, minimum, maximum) {
  return Math.max(minimum, Math.min(maximum, value));
}

async function analyzeSelection(bytes, generation) {
  drawLoading(ui.waveform, "DECODING WAVEFORM");
  drawLoading(ui.spectrogram, "COMPUTING SPECTRAL FIELD");
  const context = new AudioContext();
  try {
    const decoded = await context.decodeAudioData(bytes);
    if (generation !== state.selectionGeneration) return;
    state.analysisSamples = decoded.getChannelData(0);
    state.analysisSampleRate = decoded.sampleRate;
    resizeVisualizations();
  } finally {
    await context.close();
  }
}

function resizeVisualizations() {
  if (!state.analysisSamples || !state.analysisSampleRate) return;
  state.waveformLayer = renderWaveformLayer(ui.waveform, state.analysisSamples);
  state.spectrumLayer = renderSpectrogramLayer(
    ui.spectrogram,
    state.analysisSamples,
    state.analysisSampleRate,
  );
  ui.waveform.setAttribute("aria-busy", "false");
  ui.spectrogram.setAttribute("aria-busy", "false");
}

function renderWaveformLayer(canvas, samples) {
  const layer = sizedLayer(canvas);
  const context = layer.getContext("2d");
  const { width, height } = layer;
  context.fillStyle = "#181a19";
  context.fillRect(0, 0, width, height);
  context.fillStyle = "#929e7c";
  const stride = samples.length / width;
  for (let x = 0; x < width; x++) {
    const start = Math.floor(x * stride);
    const end = Math.max(start + 1, Math.floor((x + 1) * stride));
    let minimum = 1;
    let maximum = -1;
    for (let index = start; index < end && index < samples.length; index++) {
      minimum = Math.min(minimum, samples[index]);
      maximum = Math.max(maximum, samples[index]);
    }
    const top = (1 - maximum) * height * 0.5;
    const bottom = (1 - minimum) * height * 0.5;
    context.fillRect(x, top, 1, Math.max(1, bottom - top));
  }
  return layer;
}

function renderSpectrogramLayer(canvas, samples, sampleRate) {
  const layer = sizedLayer(canvas);
  const context = layer.getContext("2d");
  const { width, height } = layer;
  // One analysis column per CSS pixel retains all visible time detail while
  // avoiding duplicate FFT work solely for a high-DPI backing store.
  const columns = Math.min(Math.max(1, Math.ceil(canvas.clientWidth || width)), 2880);
  const fftSize = 16384;
  canvas.dataset.fftSize = String(fftSize);
  canvas.dataset.analysisColumns = String(columns);
  const real = new Float64Array(fftSize);
  const imaginary = new Float64Array(fftSize);
  const magnitudes = new Float32Array((fftSize / 2 + 1) * columns);
  let maximum = -Infinity;
  for (let column = 0; column < columns; column++) {
    const center = Math.floor((column / Math.max(1, columns - 1)) * (samples.length - 1));
    const start = center - fftSize / 2;
    for (let index = 0; index < fftSize; index++) {
      const sample = samples[Math.max(0, Math.min(samples.length - 1, start + index))];
      real[index] = sample * (0.5 - 0.5 * Math.cos((2 * Math.PI * index) / (fftSize - 1)));
      imaginary[index] = 0;
    }
    fft(real, imaginary);
    for (let bin = 0; bin <= fftSize / 2; bin++) {
      const db = 20 * Math.log10(Math.hypot(real[bin], imaginary[bin]) + 1e-9);
      magnitudes[column * (fftSize / 2 + 1) + bin] = db;
      maximum = Math.max(maximum, db);
    }
  }
  const image = context.createImageData(width, height);
  const minimumFrequency = 50;
  const maximumFrequency = Math.min(20000, sampleRate / 2);
  for (let y = 0; y < height; y++) {
    const phase = 1 - y / Math.max(1, height - 1);
    const frequency = minimumFrequency * Math.pow(maximumFrequency / minimumFrequency, phase);
    const bin = Math.min(fftSize / 2, Math.round((frequency * fftSize) / sampleRate));
    for (let x = 0; x < width; x++) {
      const column = Math.min(columns - 1, Math.floor((x / width) * columns));
      const db = magnitudes[column * (fftSize / 2 + 1) + bin];
      const intensity = Math.max(0, Math.min(1, (db - (maximum - 72)) / 72));
      const [red, green, blue] = spectralColor(intensity);
      const offset = (y * width + x) * 4;
      image.data[offset] = red;
      image.data[offset + 1] = green;
      image.data[offset + 2] = blue;
      image.data[offset + 3] = 255;
    }
  }
  context.putImageData(image, 0, 0);
  return layer;
}

function spectralColor(value) {
  const stops = [
    [0.0, 7, 9, 10],
    [0.28, 20, 49, 54],
    [0.55, 44, 129, 124],
    [0.78, 216, 241, 80],
    [1.0, 255, 114, 92],
  ];
  for (let index = 1; index < stops.length; index++) {
    if (value <= stops[index][0]) {
      const left = stops[index - 1];
      const right = stops[index];
      const phase = (value - left[0]) / (right[0] - left[0]);
      return [1, 2, 3].map((channel) => Math.round(left[channel] + phase * (right[channel] - left[channel])));
    }
  }
  return stops.at(-1).slice(1);
}

function fft(real, imaginary) {
  const length = real.length;
  for (let i = 1, j = 0; i < length; i++) {
    let bit = length >> 1;
    for (; j & bit; bit >>= 1) j ^= bit;
    j ^= bit;
    if (i < j) {
      [real[i], real[j]] = [real[j], real[i]];
      [imaginary[i], imaginary[j]] = [imaginary[j], imaginary[i]];
    }
  }
  for (let size = 2; size <= length; size <<= 1) {
    const angle = (-2 * Math.PI) / size;
    const stepReal = Math.cos(angle);
    const stepImaginary = Math.sin(angle);
    for (let start = 0; start < length; start += size) {
      let rotationReal = 1;
      let rotationImaginary = 0;
      for (let offset = 0; offset < size / 2; offset++) {
        const even = start + offset;
        const odd = even + size / 2;
        const oddReal = real[odd] * rotationReal - imaginary[odd] * rotationImaginary;
        const oddImaginary = real[odd] * rotationImaginary + imaginary[odd] * rotationReal;
        real[odd] = real[even] - oddReal;
        imaginary[odd] = imaginary[even] - oddImaginary;
        real[even] += oddReal;
        imaginary[even] += oddImaginary;
        const nextReal = rotationReal * stepReal - rotationImaginary * stepImaginary;
        rotationImaginary = rotationReal * stepImaginary + rotationImaginary * stepReal;
        rotationReal = nextReal;
      }
    }
  }
}

function animateCursor() {
  const duration = ui.audio.duration || state.catalog.input_seconds;
  const phase = Math.max(0, Math.min(1, ui.audio.currentTime / duration));
  drawLayerWithCursor(ui.waveform, state.waveformLayer, phase);
  drawLayerWithCursor(ui.spectrogram, state.spectrumLayer, phase);
  requestAnimationFrame(animateCursor);
}

function drawLayerWithCursor(canvas, layer, phase) {
  if (!layer) return;
  const context = canvas.getContext("2d");
  if (canvas.width !== layer.width || canvas.height !== layer.height) {
    canvas.width = layer.width;
    canvas.height = layer.height;
  }
  context.drawImage(layer, 0, 0);
  const x = Math.round(phase * (canvas.width - 1)) + 0.5;
  context.strokeStyle = "#c77e69";
  context.lineWidth = Math.max(1, window.devicePixelRatio);
  context.beginPath();
  context.moveTo(x, 0);
  context.lineTo(x, canvas.height);
  context.stroke();
}

function sizedLayer(canvas) {
  const layer = document.createElement("canvas");
  const ratio = Math.min(window.devicePixelRatio || 1, 2);
  layer.width = Math.max(300, Math.floor(canvas.clientWidth * ratio));
  const cssHeight = canvas.clientHeight || Number(canvas.getAttribute("height"));
  layer.height = Math.max(80, Math.floor(cssHeight * ratio));
  canvas.width = layer.width;
  canvas.height = layer.height;
  return layer;
}

function drawLoading(canvas, label) {
  canvas.setAttribute("aria-busy", "true");
  const layer = sizedLayer(canvas);
  const context = layer.getContext("2d");
  context.fillStyle = "#090b0b";
  context.fillRect(0, 0, layer.width, layer.height);
  context.fillStyle = "#777";
  context.font = `${11 * (window.devicePixelRatio || 1)}px monospace`;
  context.fillText(label, 14, 24);
  canvas.getContext("2d").drawImage(layer, 0, 0);
}

function refreshButtons() {
  for (const button of ui.algorithmButtons.querySelectorAll("button")) {
    const active = button.dataset.value === state.algorithm;
    button.classList.toggle("active", active);
    button.setAttribute("aria-pressed", active);
  }
}

async function togglePlayback() {
  if (!state.audioReady) return;
  if (ui.audio.paused) {
    await ui.audio.play();
    hideError();
  } else {
    ui.audio.pause();
  }
}

function refreshPlayButton() {
  if (!state.audioReady) {
    ui.playButton.textContent = "▶";
    ui.playButton.setAttribute("aria-label", "Play");
    ui.playButton.title =
      "Playback becomes available after the current convolution and its visualizations are ready.";
    return;
  }
  ui.playButton.textContent = ui.audio.paused ? "▶" : "❚❚";
  ui.playButton.setAttribute("aria-label", ui.audio.paused ? "Play" : "Pause");
  ui.playButton.title = ui.audio.paused
    ? "Play the current in-memory convolution from its present position. Playback loops automatically at the end."
    : "Pause playback at the current position. The rendered audio remains in memory and can resume from this point.";
}

function setTransportReady(ready) {
  state.audioReady = ready;
  ui.playButton.disabled = !ready;
  ui.seek.disabled = !ready;
  refreshPlayButton();
}

function refreshTransport() {
  const duration = Number.isFinite(ui.audio.duration) ? ui.audio.duration : 0;
  ui.seek.max = duration;
  if (!ui.seek.matches(":active")) ui.seek.value = ui.audio.currentTime;
  ui.currentTime.textContent = formatTime(ui.audio.currentTime);
  ui.duration.textContent = formatTime(duration);
}

function formatTime(seconds) {
  const safe = Math.max(0, Number.isFinite(seconds) ? seconds : 0);
  const minutes = Math.floor(safe / 60);
  const remainder = (safe - minutes * 60).toFixed(3).padStart(6, "0");
  return `${minutes}:${remainder}`;
}

function shortAlgorithm(value) {
  return {
    windowed_convolution: "windowed",
    evolving_ir: "ir",
    chunk_crossfade: "chunks",
    full_convolution: "full",
    dry_a: "dry a",
    dry_b: "dry b",
  }[value];
}

function once(target, eventName) {
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      target.removeEventListener(eventName, resolveEvent);
      target.removeEventListener("error", rejectEvent);
    };
    const resolveEvent = (event) => {
      cleanup();
      resolve(event);
    };
    const rejectEvent = () => {
      cleanup();
      const code = target.error?.code;
      const reason = {
        1: "playback was aborted",
        2: "a network error interrupted the media load",
        3: "the WAV could not be decoded",
        4: "the media source is not supported",
      }[code];
      reject(
        new Error(
          target.error?.message ||
            (reason
              ? `Audio failed: ${reason} (media error ${code})`
              : `Failed while waiting for ${eventName}`),
        ),
      );
    };
    target.addEventListener(eventName, resolveEvent, { once: true });
    target.addEventListener("error", rejectEvent, { once: true });
  });
}

function debounce(callback, delay) {
  let timer;
  return (...arguments_) => {
    clearTimeout(timer);
    timer = setTimeout(() => callback(...arguments_), delay);
  };
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function showError(error) {
  ui.errorPanel.hidden = false;
  ui.errorPanel.textContent = error instanceof Error ? error.stack || error.message : String(error);
}

function hideError() {
  ui.errorPanel.hidden = true;
  ui.errorPanel.textContent = "";
}

boot();
