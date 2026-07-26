import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const appDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = await readFile(resolve(appDir, "src/spectrum-worker.js"), "utf8");
let response;
const worker = {
  onmessage: null,
  postMessage: (message) => {
    response = message;
  },
};
vm.runInNewContext(source, {
  Float32Array,
  Math,
  Uint16Array,
  Uint32Array,
  performance,
  self: worker,
});
assert.equal(typeof worker.onmessage, "function", "spectrum worker installs its handler");

const sampleRate = 48_000;
const fftSize = 16_384;
worker.onmessage({ data: { type: "warm", fftSize } });
assert.equal(response.type, "warm");
assert.equal(response.algorithm, "packed-real-radix2");
const targetBin = 150;
const targetFrequency = (targetBin * sampleRate) / fftSize;
const samples = new Float32Array(sampleRate);
for (let index = 0; index < samples.length; index++) {
  samples[index] =
    0.4 * Math.sin((2 * Math.PI * targetFrequency * index) / sampleRate);
}
const comparisonBin = Math.round((1_200 * fftSize) / sampleRate);
const rowBins = new Uint16Array([targetBin, comparisonBin]);
worker.onmessage({
  data: {
    columnStart: 0,
    columnEnd: 3,
    totalColumns: 3,
    fftSize,
    rowBins: rowBins.buffer,
    sampleRate,
    totalSamples: samples.length,
    sampleStart: 0,
    samples: samples.buffer,
  },
});
assert.ok(response, "spectrum worker returns a stripe");
assert.equal(response.algorithm, "packed-real-radix2");
assert.equal(response.complexFftSize, fftSize / 2);
assert.ok(
  response.butterflyReduction > 2.1,
  `packed-real FFT should remove over half the complex butterflies: ${response.butterflyReduction}`,
);
const values = new Float32Array(response.values);
assert.equal(values.length, 6);
assert.ok(values.every(Number.isFinite), "spectrum stripe remains finite at padded edges");
const middleTarget = values[2];
const middleComparison = values[3];
assert.ok(
  middleTarget > middleComparison + 50,
  `${targetFrequency.toFixed(2)} Hz landmark should dominate 1.2 kHz: ` +
    `${middleTarget.toFixed(1)} vs ${middleComparison.toFixed(1)} dB`,
);
assert.ok(response.computeMs > 0, "spectrum worker reports its compute time");

const exactFftSize = 64;
const exactSamples = new Float32Array(exactFftSize * 2 + 1);
for (let index = 0; index < exactSamples.length; index++) {
  exactSamples[index] =
    0.23 * Math.sin(index * 0.37) +
    0.11 * Math.cos(index * 1.19) +
    ((index * 17) % 29) / 100;
}
const exactBins = Uint16Array.from(
  { length: exactFftSize / 2 + 1 },
  (_, index) => index,
);
worker.onmessage({
  data: {
    columnStart: 1,
    columnEnd: 2,
    totalColumns: 3,
    fftSize: exactFftSize,
    rowBins: exactBins.buffer,
    sampleRate,
    totalSamples: exactSamples.length,
    sampleStart: 0,
    samples: exactSamples.buffer,
  },
});
const exactValues = new Float32Array(response.values);
const frameStart = Math.floor((exactSamples.length - 1) / 2) - exactFftSize / 2;
for (let bin = 0; bin <= exactFftSize / 2; bin++) {
  let real = 0;
  let imaginary = 0;
  for (let index = 0; index < exactFftSize; index++) {
    const window =
      0.5 - 0.5 * Math.cos((2 * Math.PI * index) / (exactFftSize - 1));
    const angle = (-2 * Math.PI * bin * index) / exactFftSize;
    const sample = exactSamples[frameStart + index] * window;
    real += sample * Math.cos(angle);
    imaginary += sample * Math.sin(angle);
  }
  const expectedDb = 10 * Math.log10(real * real + imaginary * imaginary + 1e-18);
  assert.ok(
    Math.abs(exactValues[bin] - expectedDb) < 0.001,
    `packed-real bin ${bin} differs from direct DFT: ` +
      `${exactValues[bin]} vs ${expectedDb}`,
  );
}

console.log(
  `spectrum worker synthetic test passed (${targetFrequency.toFixed(2)} Hz landmark, ` +
    `${response.butterflyReduction.toFixed(2)}× fewer core butterflies)`,
);
