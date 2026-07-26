"use strict";

let cachedPlan = null;

self.onmessage = (event) => {
  if (event.data.type === "warm") {
    const started = performance.now();
    const plan = spectrumPlan(event.data.fftSize);
    if (!plan.warmed) {
      plan.real.fill(0);
      plan.imaginary.fill(0);
      fft(
        plan.real,
        plan.imaginary,
        plan.bitReverse,
        plan.twiddleReal,
        plan.twiddleImaginary,
      );
      plan.warmed = true;
    }
    self.postMessage({
      type: "warm",
      computeMs: performance.now() - started,
      algorithm: "packed-real-radix2",
    });
    return;
  }
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
  const plan = spectrumPlan(fftSize);
  const {
    packedSize,
    real,
    imaginary,
    hann,
    bitReverse,
    twiddleReal,
    twiddleImaginary,
  } = plan;
  const recoveryReal = new Float32Array(bins.length);
  const recoveryImaginary = new Float32Array(bins.length);
  for (let row = 0; row < bins.length; row++) {
    const angle = (-2 * Math.PI * bins[row]) / fftSize;
    recoveryReal[row] = Math.cos(angle);
    recoveryImaginary[row] = Math.sin(angle);
  }
  let maximum = -Infinity;
  for (let column = columnStart; column < columnEnd; column++) {
    const center = Math.floor(
      (column / Math.max(1, totalColumns - 1)) * (totalSamples - 1),
    );
    const frameStart = center - fftSize / 2;
    for (let index = 0; index < packedSize; index++) {
      const evenIndex = index * 2;
      const evenSource = frameStart + evenIndex;
      const oddSource = evenSource + 1;
      const evenSample = evenSource < 0
        ? 0
        : evenSource >= totalSamples
          ? 0
          : samples[evenSource - sampleStart] || 0;
      const oddSample = oddSource < 0
        ? 0
        : oddSource >= totalSamples
          ? 0
          : samples[oddSource - sampleStart] || 0;
      real[index] = evenSample * hann[evenIndex];
      imaginary[index] = oddSample * hann[evenIndex + 1];
    }
    fft(real, imaginary, bitReverse, twiddleReal, twiddleImaginary);
    const localColumn = column - columnStart;
    for (let row = 0; row < bins.length; row++) {
      const bin = bins[row];
      const packedBin = bin % packedSize;
      const mirrorBin = (packedSize - packedBin) % packedSize;
      const aReal = real[packedBin];
      const aImaginary = imaginary[packedBin];
      const bReal = real[mirrorBin];
      const bImaginary = -imaginary[mirrorBin];
      const differenceReal = aReal - bReal;
      const differenceImaginary = aImaginary - bImaginary;
      const rotatedReal =
        recoveryReal[row] * differenceReal -
        recoveryImaginary[row] * differenceImaginary;
      const rotatedImaginary =
        recoveryReal[row] * differenceImaginary +
        recoveryImaginary[row] * differenceReal;
      const realPart = 0.5 * (aReal + bReal + rotatedImaginary);
      const imaginaryPart =
        0.5 * (aImaginary + bImaginary - rotatedReal);
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
      type: "spectrum",
      algorithm: "packed-real-radix2",
      complexFftSize: packedSize,
      butterflyReduction:
        (2 * Math.log2(fftSize)) / Math.log2(packedSize),
      values: values.buffer,
    },
    [values.buffer],
  );
};

function spectrumPlan(fftSize) {
  if (cachedPlan?.fftSize === fftSize) return cachedPlan;
  const packedSize = fftSize / 2;
  const real = new Float32Array(packedSize);
  const imaginary = new Float32Array(packedSize);
  const hann = new Float32Array(fftSize);
  const bitReverse = new Uint32Array(packedSize);
  const fftBits = Math.log2(packedSize);
  const twiddleReal = new Float32Array(packedSize / 2);
  const twiddleImaginary = new Float32Array(packedSize / 2);
  for (let index = 0; index < fftSize; index++) {
    hann[index] =
      0.5 - 0.5 * Math.cos((2 * Math.PI * index) / (fftSize - 1));
  }
  for (let index = 0; index < packedSize; index++) {
    bitReverse[index] = reverseBits(index, fftBits);
    if (index < packedSize / 2) {
      const angle = (-2 * Math.PI * index) / packedSize;
      twiddleReal[index] = Math.cos(angle);
      twiddleImaginary[index] = Math.sin(angle);
    }
  }
  cachedPlan = {
    fftSize,
    packedSize,
    real,
    imaginary,
    hann,
    bitReverse,
    twiddleReal,
    twiddleImaginary,
    warmed: false,
  };
  return cachedPlan;
}

function reverseBits(value, bits) {
  let reversed = 0;
  for (let bit = 0; bit < bits; bit++) {
    reversed = (reversed << 1) | (value & 1);
    value >>>= 1;
  }
  return reversed;
}

function fft(
  real,
  imaginary,
  bitReverse,
  twiddleReal,
  twiddleImaginary,
) {
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
    const twiddleStep = length / size;
    for (let start = 0; start < length; start += size) {
      for (let offset = 0; offset < size / 2; offset++) {
        const even = start + offset;
        const odd = even + size / 2;
        const twiddleIndex = offset * twiddleStep;
        const rotationReal = twiddleReal[twiddleIndex];
        const rotationImaginary = twiddleImaginary[twiddleIndex];
        const oddReal =
          real[odd] * rotationReal - imaginary[odd] * rotationImaginary;
        const oddImaginary =
          real[odd] * rotationImaginary + imaginary[odd] * rotationReal;
        real[odd] = real[even] - oddReal;
        imaginary[odd] = imaginary[even] - oddImaginary;
        real[even] += oddReal;
        imaginary[even] += oddImaginary;
      }
    }
  }
}
