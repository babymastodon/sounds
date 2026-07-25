const ui = {
  matrixStatus: document.querySelector("#matrixStatus"),
  pairSelect: document.querySelector("#pairSelect"),
  algorithmButtons: document.querySelector("#algorithmButtons"),
  presetButtons: document.querySelector("#presetButtons"),
  sourceA: document.querySelector("#sourceA"),
  sourceB: document.querySelector("#sourceB"),
  renderTitle: document.querySelector("#renderTitle"),
  metrics: document.querySelector("#metrics"),
  audio: document.querySelector("#audio"),
  playButton: document.querySelector("#playButton"),
  seek: document.querySelector("#seek"),
  volume: document.querySelector("#volume"),
  currentTime: document.querySelector("#currentTime"),
  duration: document.querySelector("#duration"),
  waveform: document.querySelector("#waveform"),
  spectrogram: document.querySelector("#spectrogram"),
  windowReadout: document.querySelector("#windowReadout"),
  errorPanel: document.querySelector("#errorPanel"),
};

const state = {
  catalog: null,
  outputDir: "",
  pairs: [],
  pair: "",
  algorithm: "multiresolution",
  preset: "short",
  waveformLayer: null,
  spectrumLayer: null,
  loadGeneration: 0,
};

async function boot() {
  try {
    const tauri = window.__TAURI__;
    if (!tauri?.core?.invoke || !tauri?.core?.convertFileSrc) {
      throw new Error("This interface must be opened through the Tauri application.");
    }
    const bootstrap = await tauri.core.invoke("load_bootstrap");
    state.catalog = bootstrap.catalog;
    state.outputDir = bootstrap.outputDir;
    state.pairs = [...new Set(state.catalog.clips.map((clip) => clip.pair))].sort();
    if (!state.pairs.length) {
      throw new Error("The catalog has no rendered clips.");
    }
    state.pair = state.pairs[0];
    buildControls();
    bindEvents();
    ui.matrixStatus.textContent =
      `${state.catalog.clips.length} / ${state.catalog.expected_outputs} RENDERS`;
    await selectClip(false);
    requestAnimationFrame(animateCursor);
  } catch (error) {
    showError(error);
  }
}

function buildControls() {
  const sourceById = new Map(state.catalog.sources.map((source) => [source.id, source]));
  for (const pair of state.pairs) {
    const [left, right] = pair.split("__");
    const option = document.createElement("option");
    option.value = pair;
    option.textContent =
      `${sourceById.get(left)?.category ?? left} × ${sourceById.get(right)?.category ?? right}`;
    ui.pairSelect.append(option);
  }
  ui.pairSelect.value = state.pair;

  for (const algorithm of state.catalog.algorithms) {
    const button = document.createElement("button");
    button.type = "button";
    button.dataset.value = algorithm.id;
    button.title = `${algorithm.title} · expected quality rank ${algorithm.rank}`;
    button.textContent = `${algorithm.rank}. ${shortAlgorithm(algorithm.id)}`;
    ui.algorithmButtons.append(button);
  }
  for (const preset of state.catalog.presets) {
    const button = document.createElement("button");
    button.type = "button";
    button.dataset.value = preset.id;
    button.textContent = preset.id;
    ui.presetButtons.append(button);
  }
  refreshButtons();
}

function bindEvents() {
  ui.pairSelect.addEventListener("change", () => {
    state.pair = ui.pairSelect.value;
    selectClip(true);
  });
  ui.algorithmButtons.addEventListener("click", (event) => {
    const button = event.target.closest("button");
    if (!button) return;
    state.algorithm = button.dataset.value;
    refreshButtons();
    selectClip(true);
  });
  ui.presetButtons.addEventListener("click", (event) => {
    const button = event.target.closest("button");
    if (!button) return;
    state.preset = button.dataset.value;
    refreshButtons();
    selectClip(true);
  });
  ui.playButton.addEventListener("click", togglePlayback);
  ui.audio.addEventListener("play", refreshPlayButton);
  ui.audio.addEventListener("pause", refreshPlayButton);
  ui.audio.addEventListener("loadedmetadata", refreshTransport);
  ui.audio.addEventListener("timeupdate", refreshTransport);
  ui.audio.addEventListener("ended", refreshPlayButton);
  ui.seek.addEventListener("input", () => {
    ui.audio.currentTime = Number(ui.seek.value);
  });
  ui.volume.addEventListener("input", () => {
    ui.audio.volume = Number(ui.volume.value);
  });
  ui.audio.volume = Number(ui.volume.value);
  for (const canvas of [ui.waveform, ui.spectrogram]) {
    canvas.addEventListener("click", (event) => {
      const bounds = canvas.getBoundingClientRect();
      const phase = Math.max(0, Math.min(1, (event.clientX - bounds.left) / bounds.width));
      ui.audio.currentTime = phase * (ui.audio.duration || 60);
    });
  }
  window.addEventListener("resize", debounce(() => selectClip(false), 180));
  document.addEventListener("keydown", (event) => {
    if (event.target.matches("select, input")) return;
    if (event.code === "Space") {
      event.preventDefault();
      togglePlayback();
    } else if (event.code === "ArrowLeft") {
      ui.audio.currentTime = Math.max(0, ui.audio.currentTime - 5);
    } else if (event.code === "ArrowRight") {
      ui.audio.currentTime = Math.min(ui.audio.duration || 60, ui.audio.currentTime + 5);
    }
  });
}

async function selectClip(preservePlayback) {
  const clip = state.catalog.clips.find(
    (candidate) =>
      candidate.pair === state.pair &&
      candidate.algorithm === state.algorithm &&
      candidate.preset === state.preset,
  );
  if (!clip) {
    showError(`No rendered file for ${state.pair} / ${state.algorithm} / ${state.preset}`);
    return;
  }
  hideError();
  const oldDuration = Number.isFinite(ui.audio.duration) ? ui.audio.duration : 60;
  const phase = preservePlayback ? ui.audio.currentTime / oldDuration : 0;
  const resume = preservePlayback && !ui.audio.paused;
  const separator = state.outputDir.includes("\\") ? "\\" : "/";
  const absolutePath = `${state.outputDir}${separator}${clip.path.replaceAll("/", separator)}`;
  const url = window.__TAURI__.core.convertFileSrc(absolutePath);
  ui.audio.src = url;
  ui.audio.load();
  await once(ui.audio, "loadedmetadata");
  ui.audio.currentTime = Math.max(0, Math.min(ui.audio.duration - 0.01, phase * ui.audio.duration));
  if (resume) {
    await ui.audio.play();
  }
  updateMetadata(clip);
  await analyzeSelection(url);
}

function updateMetadata(clip) {
  const sources = new Map(state.catalog.sources.map((source) => [source.id, source]));
  const left = sources.get(clip.left);
  const right = sources.get(clip.right);
  ui.sourceA.innerHTML = sourceMarkup("A", left);
  ui.sourceB.innerHTML = sourceMarkup("B", right);
  const algorithm = state.catalog.algorithms.find((item) => item.id === state.algorithm);
  const preset = state.catalog.presets.find((item) => item.id === state.preset);
  ui.renderTitle.textContent = `${algorithm.title} · ${preset.title}`;
  ui.metrics.innerHTML = [
    metricMarkup("RMS", `${clip.metrics.rms_dbfs.toFixed(1)} dBFS`),
    metricMarkup("PEAK", `${(clip.metrics.peak * 100).toFixed(1)}%`),
    metricMarkup("DC", clip.metrics.dc_offset.toExponential(1)),
  ].join("");
  ui.windowReadout.textContent =
    `A ${preset.clip_a_seconds.toFixed(2)}s · B ${preset.clip_b_seconds.toFixed(2)}s · HOP ${preset.hop_seconds.toFixed(2)}s`;
}

function sourceMarkup(label, source) {
  return `<span class="field-label">SOURCE ${label} · ${escapeHtml(source.kind)}</span>
    <strong>${escapeHtml(source.category)}</strong>
    <small>${escapeHtml(source.creator)} · ${escapeHtml(source.license)} ·
      <a href="${escapeHtml(source.source_page)}">source</a></small>`;
}

function metricMarkup(label, value) {
  return `<div><dt>${label}</dt><dd>${value}</dd></div>`;
}

async function analyzeSelection(url) {
  const generation = ++state.loadGeneration;
  drawLoading(ui.waveform, "DECODING WAVEFORM");
  drawLoading(ui.spectrogram, "COMPUTING SPECTRAL FIELD");
  const response = await fetch(url);
  if (!response.ok) throw new Error(`Audio request failed: ${response.status}`);
  const bytes = await response.arrayBuffer();
  const context = new AudioContext();
  try {
    const decoded = await context.decodeAudioData(bytes);
    if (generation !== state.loadGeneration) return;
    const samples = decoded.getChannelData(0);
    state.waveformLayer = renderWaveformLayer(ui.waveform, samples);
    state.spectrumLayer = renderSpectrogramLayer(ui.spectrogram, samples, decoded.sampleRate);
  } finally {
    await context.close();
  }
}

function renderWaveformLayer(canvas, samples) {
  const layer = sizedLayer(canvas);
  const context = layer.getContext("2d");
  const { width, height } = layer;
  context.fillStyle = "#090b0b";
  context.fillRect(0, 0, width, height);
  context.strokeStyle = "rgba(243,239,226,.1)";
  context.beginPath();
  context.moveTo(0, height / 2);
  context.lineTo(width, height / 2);
  context.stroke();
  context.fillStyle = "#d8f150";
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
  const columns = Math.min(width, 720);
  const fftSize = 2048;
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
  const duration = ui.audio.duration || 60;
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
  context.strokeStyle = "#ff725c";
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
  layer.height = Math.floor(Number(canvas.getAttribute("height")) * ratio);
  canvas.width = layer.width;
  canvas.height = layer.height;
  return layer;
}

function drawLoading(canvas, label) {
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
    button.classList.toggle("active", button.dataset.value === state.algorithm);
  }
  for (const button of ui.presetButtons.querySelectorAll("button")) {
    button.classList.toggle("active", button.dataset.value === state.preset);
  }
}

async function togglePlayback() {
  if (ui.audio.paused) await ui.audio.play();
  else ui.audio.pause();
}

function refreshPlayButton() {
  ui.playButton.textContent = ui.audio.paused ? "▶" : "❚❚";
  ui.playButton.setAttribute("aria-label", ui.audio.paused ? "Play" : "Pause");
}

function refreshTransport() {
  const duration = Number.isFinite(ui.audio.duration) ? ui.audio.duration : 60;
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
    multiresolution: "MULTI",
    sliding_wola: "WOLA",
    evolving_ir: "IR",
    chunk_crossfade: "CHUNKS",
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
      reject(new Error(target.error?.message || `Failed while waiting for ${eventName}`));
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
}

boot();

