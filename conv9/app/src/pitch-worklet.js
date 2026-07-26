"use strict";

const FFT_SIZE = 2048;
const HOP_SIZE = FFT_SIZE / 4;
const RING_SIZE = FFT_SIZE * 8;
const TWO_PI = 2 * Math.PI;

class PitchPreserver extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const requestedChannels = Number(options.processorOptions?.channels) || 1;
    this.channels = Math.max(1, Math.min(2, requestedChannels));
    this.factor = Math.max(
      0.5,
      Math.min(2, Number(options.processorOptions?.factor) || 1),
    );
    this.inputTotal = 0;
    this.outputTotal = 0;
    this.nextFrameEnd = FFT_SIZE;
    this.window = new Float32Array(FFT_SIZE);
    this.bitReverse = new Uint16Array(FFT_SIZE);
    const bits = Math.log2(FFT_SIZE);
    for (let index = 0; index < FFT_SIZE; index++) {
      this.window[index] = Math.sqrt(
        0.5 - 0.5 * Math.cos((TWO_PI * index) / FFT_SIZE),
      );
      this.bitReverse[index] = reverseBits(index, bits);
    }
    this.channelState = Array.from({ length: this.channels }, () => ({
      input: new Float32Array(RING_SIZE),
      output: new Float32Array(RING_SIZE),
      real: new Float32Array(FFT_SIZE),
      imaginary: new Float32Array(FFT_SIZE),
      magnitudes: new Float32Array(FFT_SIZE / 2 + 1),
      frequencies: new Float32Array(FFT_SIZE / 2 + 1),
      previousPhase: new Float32Array(FFT_SIZE / 2 + 1),
      synthesisPhase: new Float32Array(FFT_SIZE / 2 + 1),
      initialized: false,
    }));
    this.port.postMessage({
      type: "ready",
      latencyFrames: FFT_SIZE,
    });
  }

  process(inputs, outputs) {
    const input = inputs[0];
    const output = outputs[0];
    if (!output?.length) return true;
    const frames = output[0].length;
    for (let frame = 0; frame < frames; frame++) {
      const inputIndex = this.inputTotal % RING_SIZE;
      for (let channel = 0; channel < this.channels; channel++) {
        const source = input[channel] || input[0];
        this.channelState[channel].input[inputIndex] = source?.[frame] || 0;
      }
      this.inputTotal++;
    }

    while (this.inputTotal >= this.nextFrameEnd) {
      for (let channel = 0; channel < this.channels; channel++) {
        this.processFrame(this.channelState[channel], this.nextFrameEnd);
      }
      this.nextFrameEnd += HOP_SIZE;
    }

    for (let frame = 0; frame < frames; frame++) {
      const outputIndex = this.outputTotal % RING_SIZE;
      for (let channel = 0; channel < output.length; channel++) {
        const channelState =
          this.channelState[Math.min(channel, this.channelState.length - 1)];
        output[channel][frame] = channelState.output[outputIndex];
        channelState.output[outputIndex] = 0;
      }
      this.outputTotal++;
    }
    return true;
  }

  processFrame(channel, frameEnd) {
    const {
      input,
      output,
      real,
      imaginary,
      magnitudes,
      frequencies,
      previousPhase,
      synthesisPhase,
    } = channel;
    const frameStart = frameEnd - FFT_SIZE;
    let spectralChange = 0;
    let spectralEnergy = 1e-9;
    for (let index = 0; index < FFT_SIZE; index++) {
      real[index] = input[(frameStart + index) % RING_SIZE] * this.window[index];
      imaginary[index] = 0;
    }
    fft(real, imaginary, this.bitReverse, false);
    for (let bin = 0; bin <= FFT_SIZE / 2; bin++) {
      const magnitude = Math.hypot(real[bin], imaginary[bin]);
      const phase = Math.atan2(imaginary[bin], real[bin]);
      const expected = (TWO_PI * bin * HOP_SIZE) / FFT_SIZE;
      const deviation = wrapPhase(phase - previousPhase[bin] - expected);
      frequencies[bin] =
        (TWO_PI * bin) / FFT_SIZE + deviation / HOP_SIZE;
      spectralChange += Math.max(0, magnitude - magnitudes[bin]);
      spectralEnergy += magnitude;
      magnitudes[bin] = magnitude;
      previousPhase[bin] = phase;
    }
    const transient = channel.initialized && spectralChange / spectralEnergy > 0.42;
    real.fill(0);
    imaginary.fill(0);
    const maximumBin = FFT_SIZE / 2;
    for (let targetBin = 0; targetBin <= maximumBin; targetBin++) {
      const sourceBin = targetBin / this.factor;
      if (sourceBin > maximumBin) continue;
      const lower = Math.floor(sourceBin);
      const upper = Math.min(maximumBin, lower + 1);
      const mix = sourceBin - lower;
      const magnitude =
        magnitudes[lower] + mix * (magnitudes[upper] - magnitudes[lower]);
      const frequency =
        frequencies[lower] + mix * (frequencies[upper] - frequencies[lower]);
      if (!channel.initialized || transient) {
        const sourcePhase = interpolatePhase(
          previousPhase[lower],
          previousPhase[upper],
          mix,
        );
        synthesisPhase[targetBin] = sourcePhase;
      } else {
        synthesisPhase[targetBin] = wrapPhase(
          synthesisPhase[targetBin] + frequency * this.factor * HOP_SIZE,
        );
      }
      real[targetBin] = magnitude * Math.cos(synthesisPhase[targetBin]);
      imaginary[targetBin] = magnitude * Math.sin(synthesisPhase[targetBin]);
      if (targetBin > 0 && targetBin < maximumBin) {
        real[FFT_SIZE - targetBin] = real[targetBin];
        imaginary[FFT_SIZE - targetBin] = -imaginary[targetBin];
      }
    }
    channel.initialized = true;
    fft(real, imaginary, this.bitReverse, true);

    // Four root-Hann frames overlap. Their analysis/synthesis windows multiply
    // to Hann, whose four-way overlap sum is exactly two.
    const outputStart = frameStart + FFT_SIZE;
    for (let index = 0; index < FFT_SIZE; index++) {
      const absolute = outputStart + index;
      output[absolute % RING_SIZE] += real[index] * this.window[index] * 0.5;
    }
  }
}

function interpolatePhase(left, right, mix) {
  return wrapPhase(left + wrapPhase(right - left) * mix);
}

function wrapPhase(value) {
  return value - TWO_PI * Math.round(value / TWO_PI);
}

function reverseBits(value, bits) {
  let reversed = 0;
  for (let bit = 0; bit < bits; bit++) {
    reversed = (reversed << 1) | (value & 1);
    value >>>= 1;
  }
  return reversed;
}

function fft(real, imaginary, bitReverse, inverse) {
  const length = real.length;
  for (let index = 0; index < length; index++) {
    const reversed = bitReverse[index];
    if (index < reversed) {
      const realValue = real[index];
      real[index] = real[reversed];
      real[reversed] = realValue;
      const imaginaryValue = imaginary[index];
      imaginary[index] = imaginary[reversed];
      imaginary[reversed] = imaginaryValue;
    }
  }
  for (let size = 2; size <= length; size <<= 1) {
    const angle = (inverse ? TWO_PI : -TWO_PI) / size;
    const stepReal = Math.cos(angle);
    const stepImaginary = Math.sin(angle);
    for (let start = 0; start < length; start += size) {
      let rotationReal = 1;
      let rotationImaginary = 0;
      for (let offset = 0; offset < size / 2; offset++) {
        const even = start + offset;
        const odd = even + size / 2;
        const oddReal =
          real[odd] * rotationReal - imaginary[odd] * rotationImaginary;
        const oddImaginary =
          real[odd] * rotationImaginary + imaginary[odd] * rotationReal;
        real[odd] = real[even] - oddReal;
        imaginary[odd] = imaginary[even] - oddImaginary;
        real[even] += oddReal;
        imaginary[even] += oddImaginary;
        const nextReal =
          rotationReal * stepReal - rotationImaginary * stepImaginary;
        rotationImaginary =
          rotationReal * stepImaginary + rotationImaginary * stepReal;
        rotationReal = nextReal;
      }
    }
  }
  if (inverse) {
    for (let index = 0; index < length; index++) {
      real[index] /= length;
      imaginary[index] /= length;
    }
  }
}

registerProcessor("pitch-preserver", PitchPreserver);
