import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const appDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = await readFile(resolve(appDir, "src/pitch-worklet.js"), "utf8");
let Processor;
class AudioWorkletProcessor {
  constructor() {
    this.port = { postMessage: () => {} };
  }
}
vm.runInNewContext(source, {
  AudioWorkletProcessor,
  Float32Array,
  Math,
  registerProcessor: (name, implementation) => {
    assert.equal(name, "pitch-preserver");
    Processor = implementation;
  },
});
assert.ok(Processor, "pitch worklet registers its processor");

const sampleRate = 48_000;
const blockFrames = 128;
const seconds = 2.5;
const inputFrequency = 660;
const factor = 2 / 3;
const processor = new Processor({
  processorOptions: { channels: 1, factor },
});
const rendered = new Float32Array(
  Math.floor((sampleRate * seconds) / blockFrames) * blockFrames,
);
for (let start = 0; start < rendered.length; start += blockFrames) {
  const input = new Float32Array(blockFrames);
  const output = new Float32Array(blockFrames);
  for (let frame = 0; frame < blockFrames; frame++) {
    input[frame] =
      0.25 * Math.sin((2 * Math.PI * inputFrequency * (start + frame)) / sampleRate);
  }
  processor.process([[input]], [[output]]);
  rendered.set(output, start);
}

const stable = rendered.subarray(8_192);
assert.ok(stable.every(Number.isFinite), "pitch worklet output remains finite");
const rms = Math.sqrt(
  stable.reduce((sum, sample) => sum + sample * sample, 0) / stable.length,
);
assert.ok(rms > 0.04 && rms < 0.4, `pitch worklet has a sane output level: ${rms}`);
const dominant = dominantFrequency(stable, sampleRate, 390, 500);
assert.ok(
  Math.abs(dominant - inputFrequency * factor) <= 12,
  `660 Hz shifted by 2/3 should remain near 440 Hz, found ${dominant} Hz`,
);
assert.ok(
  goertzelPower(stable, sampleRate, dominant) >
    goertzelPower(stable, sampleRate, inputFrequency) * 4,
  "the corrected pitch is stronger than the uncorrected varispeed pitch",
);

console.log(
  `pitch worklet synthetic test passed (${inputFrequency} Hz → ${dominant} Hz, ` +
    `RMS ${rms.toFixed(3)})`,
);

function dominantFrequency(samples, rate, minimum, maximum) {
  let bestFrequency = minimum;
  let bestPower = -Infinity;
  for (let frequency = minimum; frequency <= maximum; frequency++) {
    const power = goertzelPower(samples, rate, frequency);
    if (power > bestPower) {
      bestPower = power;
      bestFrequency = frequency;
    }
  }
  return bestFrequency;
}

function goertzelPower(samples, rate, frequency) {
  const coefficient = 2 * Math.cos((2 * Math.PI * frequency) / rate);
  let previous = 0;
  let previousPrevious = 0;
  for (const sample of samples) {
    const current = sample + coefficient * previous - previousPrevious;
    previousPrevious = previous;
    previous = current;
  }
  return (
    previous * previous +
    previousPrevious * previousPrevious -
    coefficient * previous * previousPrevious
  );
}
