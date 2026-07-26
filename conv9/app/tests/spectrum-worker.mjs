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

console.log(
  `spectrum worker synthetic test passed (${targetFrequency.toFixed(2)} Hz landmark, ` +
    `${response.computeMs.toFixed(1)} ms)`,
);
