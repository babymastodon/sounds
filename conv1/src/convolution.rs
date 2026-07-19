use anyhow::{Context, Result};
use rayon::prelude::*;
use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::audio::AudioClip;

#[derive(Clone, Debug)]
pub struct PairJob {
    pub left: usize,
    pub right: usize,
    pub output_frames: usize,
    pub fft_len: usize,
}

pub struct SpectralGroup {
    pub fft_len: usize,
    pub jobs: Vec<PairJob>,
    pub spectra: HashMap<usize, Arc<[Complex32]>>,
    pub inverse: Arc<dyn ComplexToReal<f32>>,
}

pub fn make_jobs(clips: &[AudioClip]) -> Vec<PairJob> {
    let mut jobs = Vec::with_capacity(clips.len() * (clips.len() + 1) / 2);
    for left in 0..clips.len() {
        for right in left..clips.len() {
            let output_frames = clips[left].samples.len() + clips[right].samples.len() - 1;
            jobs.push(PairJob {
                left,
                right,
                output_frames,
                fft_len: output_frames.next_power_of_two(),
            });
        }
    }
    jobs
}

pub fn group_jobs(jobs: Vec<PairJob>) -> BTreeMap<usize, Vec<PairJob>> {
    let mut groups = BTreeMap::new();
    for job in jobs {
        groups.entry(job.fft_len).or_insert_with(Vec::new).push(job);
    }
    groups
}

pub fn prepare_group(
    fft_len: usize,
    jobs: Vec<PairJob>,
    clips: &[AudioClip],
) -> Result<SpectralGroup> {
    let mut planner = RealFftPlanner::<f32>::new();
    let forward = planner.plan_fft_forward(fft_len);
    let inverse = planner.plan_fft_inverse(fft_len);
    let involved = jobs
        .iter()
        .flat_map(|job| [job.left, job.right])
        .collect::<HashSet<_>>();

    let spectra = involved
        .into_par_iter()
        .map(|index| {
            let spectrum = forward_transform(&*forward, fft_len, &clips[index].samples)
                .with_context(|| format!("forward FFT for {} at {fft_len}", clips[index].id))?;
            Ok((index, Arc::<[Complex32]>::from(spectrum)))
        })
        .collect::<Result<HashMap<_, _>>>()?;

    Ok(SpectralGroup {
        fft_len,
        jobs,
        spectra,
        inverse,
    })
}

pub fn convolve_spectra(group: &SpectralGroup, job: &PairJob) -> Result<Vec<f32>> {
    let left = &group.spectra[&job.left];
    let right = &group.spectra[&job.right];
    let scale = 1.0 / group.fft_len as f32;
    let mut product = left
        .iter()
        .zip(right.iter())
        .map(|(&a, &b)| a * b * scale)
        .collect::<Vec<_>>();
    let mut output = group.inverse.make_output_vec();
    let mut scratch = group.inverse.make_scratch_vec();
    group
        .inverse
        .process_with_scratch(&mut product, &mut output, &mut scratch)
        .context("inverse FFT")?;
    output.truncate(job.output_frames);
    Ok(output)
}

fn forward_transform(
    forward: &dyn RealToComplex<f32>,
    fft_len: usize,
    input: &[f32],
) -> Result<Vec<Complex32>> {
    let mut padded = vec![0.0; fft_len];
    padded[..input.len()].copy_from_slice(input);
    let mut spectrum = forward.make_output_vec();
    let mut scratch = forward.make_scratch_vec();
    forward
        .process_with_scratch(&mut padded, &mut spectrum, &mut scratch)
        .context("forward FFT")?;
    Ok(spectrum)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn direct_convolution(a: &[f32], b: &[f32]) -> Vec<f32> {
        let mut output = vec![0.0; a.len() + b.len() - 1];
        for (i, &x) in a.iter().enumerate() {
            for (j, &y) in b.iter().enumerate() {
                output[i + j] += x * y;
            }
        }
        output
    }

    #[test]
    fn fft_convolution_matches_direct_convolution() {
        let clips = vec![
            AudioClip {
                id: "a".into(),
                samples: vec![0.25, -0.5, 1.0, 0.125],
            },
            AudioClip {
                id: "b".into(),
                samples: vec![0.3, 0.2, -0.1],
            },
        ];
        let job = PairJob {
            left: 0,
            right: 1,
            output_frames: 6,
            fft_len: 8,
        };
        let group = prepare_group(8, vec![job.clone()], &clips).unwrap();
        let actual = convolve_spectra(&group, &job).unwrap();
        let expected = direct_convolution(&clips[0].samples, &clips[1].samples);
        for (actual, expected) in actual.iter().zip(expected) {
            assert!((actual - expected).abs() < 1.0e-5, "{actual} != {expected}");
        }
    }

    #[test]
    fn job_count_is_upper_triangle_including_diagonal() {
        let clips = (0..48)
            .map(|index| AudioClip {
                id: format!("clip_{index}"),
                samples: vec![1.0],
            })
            .collect::<Vec<_>>();
        assert_eq!(make_jobs(&clips).len(), 1_176);
    }
}
