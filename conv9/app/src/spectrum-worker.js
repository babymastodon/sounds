"use strict";

self.onmessage = (event) => {
  const {
    columnStart,
    columnEnd,
    totalColumns,
    fftSize,
    rowBins,
    sampleRate,
    totalSamples,
    sampleStart,
    samples: sampleBuffer,
  } = event.data;
  const started = performance.now();
  const samples = new Float32Array(sampleBuffer);
  const bins = new Uint16Array(rowBins);
  const columnCount = columnEnd - columnStart;
  const values = new Float32Array(columnCount * bins.length);
  const real = new Float32Array(fftSize);
  const imaginary = new Float32Array(fftSize);
  const hann = new Float32Array(fftSize);
  const bitReverse = new Uint32Array(fftSize);
  const fftBits = Math.log2(fftSize);
  for (let index = 0; index < fftSize; index++) {
    hann[index] =
      0.5 - 0.5 * Math.cos((2 * Math.PI * index) / (fftSize - 1));
    bitReverse[index] = reverseBits(index, fftBits);
  }
  let maximum = -Infinity;
  for (let column = columnStart; column < columnEnd; column++) {
    const center = Math.floor(
      (column / Math.max(1, totalColumns - 1)) * (totalSamples - 1),
    );
    const frameStart = center - fftSize / 2;
    for (let index = 0; index < fftSize; index++) {
      const sourceIndex = frameStart + index;
      const sample = sourceIndex < 0
        ? 0
        : sourceIndex >= totalSamples
          ? 0
          : samples[sourceIndex - sampleStart] || 0;
      real[index] = sample * hann[index];
      imaginary[index] = 0;
    }
    fft(real, imaginary, bitReverse);
    const localColumn = column - columnStart;
    for (let row = 0; row < bins.length; row++) {
      const bin = bins[row];
      const realPart = real[bin];
      const imaginaryPart = imaginary[bin];
      const db = 10 * Math.log10(
        realPart * realPart + imaginaryPart * imaginaryPart + 1e-18,
      );
      values[localColumn * bins.length + row] = db;
      maximum = Math.max(maximum, db);
    }
  }
  self.postMessage(
    {
      columnStart,
      columnEnd,
      maximum,
      computeMs: performance.now() - started,
      sampleRate,
      values: values.buffer,
    },
    [values.buffer],
  );
};

function reverseBits(value, bits) {
  let reversed = 0;
  for (let bit = 0; bit < bits; bit++) {
    reversed = (reversed << 1) | (value & 1);
    value >>>= 1;
  }
  return reversed;
}

function fft(real, imaginary, bitReverse) {
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
    const angle = (-2 * Math.PI) / size;
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
}
