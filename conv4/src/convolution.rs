use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result};
use rayon::prelude::*;
use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

use crate::audio::{AudioClip, SAMPLE_RATE, StereoAudio};

pub const TRIM_FRACTION_OF_SHORTER: f32 = 0.5;
pub const CUT_FADE_MILLISECONDS: u32 = 20;
const CUT_FADE_FRAMES: usize = (SAMPLE_RATE as usize * CUT_FADE_MILLISECONDS as usize) / 1_000;

#[derive(Clone, Debug)]
pub struct PairJob {
    pub left: usize,
    pub right: usize,
    pub trim_frames: usize,
    pub output_frames: usize,
    pub fft_len: usize,
}

pub struct SpectralGroup {
    pub fft_len: usize,
    pub jobs: Vec<PairJob>,
    pub spectra: HashMap<usize, Arc<[Complex32]>>,
    pub forward: Arc<dyn RealToComplex<f32>>,
    pub inverse: Arc<dyn ComplexToReal<f32>>,
}

pub fn make_jobs(clips: &[AudioClip]) -> Vec<PairJob> {
    let mut jobs = Vec::with_capacity(clips.len() * (clips.len() + 1) / 2);
    for left in 0..clips.len() {
        for right in left..clips.len() {
            let left_frames = clips[left].samples.len();
            let right_frames = clips[right].samples.len();
            let trim_frames = ((left_frames.min(right_frames) as f32) * TRIM_FRACTION_OF_SHORTER)
                .round() as usize;
            let trim_frames = trim_frames.min(left_frames - 1).min(right_frames - 1);
            let output_frames = left_frames + right_frames - trim_frames - 1;
            jobs.push(PairJob {
                left,
                right,
                trim_frames,
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
        forward,
        inverse,
    })
}

pub fn convolve_stereo_spectra(
    group: &SpectralGroup,
    job: &PairJob,
    clips: &[AudioClip],
) -> Result<StereoAudio> {
    let track_1 = &group.spectra[&job.left];
    let track_2 = &group.spectra[&job.right];

    let shortened_track_2 = trim_final(&clips[job.right].samples, job.trim_frames);
    let shortened_track_2_spectrum =
        forward_transform(&*group.forward, group.fft_len, &shortened_track_2)
            .with_context(|| format!("shortened FFT for {}", clips[job.right].id))?;
    let left_channel = inverse_channel(group, job, track_1, &shortened_track_2_spectrum)?;

    let shortened_track_1 = trim_start(&clips[job.left].samples, job.trim_frames);
    let shortened_track_1_spectrum =
        forward_transform(&*group.forward, group.fft_len, &shortened_track_1)
            .with_context(|| format!("shortened FFT for {}", clips[job.left].id))?;
    let right_channel = inverse_channel(group, job, &shortened_track_1_spectrum, track_2)?;

    Ok(StereoAudio {
        left: left_channel,
        right: right_channel,
    })
}

fn trim_final(input: &[f32], trim_frames: usize) -> Vec<f32> {
    debug_assert!(trim_frames < input.len());
    let retained_frames = input.len() - trim_frames;
    let mut output = input[..retained_frames].to_vec();
    if trim_frames == 0 {
        return output;
    }

    let fade_frames = CUT_FADE_FRAMES.min(output.len());
    if fade_frames == 1 {
        output[0] = 0.0;
        return output;
    }
    let denominator = (fade_frames - 1) as f32;
    for offset in 0..fade_frames {
        let gain = 1.0 - offset as f32 / denominator;
        let index = output.len() - fade_frames + offset;
        output[index] *= gain;
    }
    output
}

fn trim_start(input: &[f32], trim_frames: usize) -> Vec<f32> {
    debug_assert!(trim_frames < input.len());
    let mut output = input[trim_frames..].to_vec();
    if trim_frames == 0 {
        return output;
    }

    let fade_frames = CUT_FADE_FRAMES.min(output.len());
    if fade_frames == 1 {
        output[0] = 0.0;
        return output;
    }
    let denominator = (fade_frames - 1) as f32;
    for (offset, sample) in output[..fade_frames].iter_mut().enumerate() {
        *sample *= offset as f32 / denominator;
    }
    output
}

fn inverse_channel(
    group: &SpectralGroup,
    job: &PairJob,
    track_1: &[Complex32],
    track_2: &[Complex32],
) -> Result<Vec<f32>> {
    let scale = 1.0 / group.fft_len as f32;
    let mut product = track_1
        .iter()
        .zip(track_2.iter())
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
    fn asymmetric_fft_convolution_matches_direct_convolution() {
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
        let job = make_jobs(&clips)
            .into_iter()
            .find(|job| job.left == 0 && job.right == 1)
            .unwrap();
        assert_eq!(job.trim_frames, 2);
        assert_eq!(job.output_frames, 4);
        let group = prepare_group(job.fft_len, vec![job.clone()], &clips).unwrap();
        let actual = convolve_stereo_spectra(&group, &job, &clips).unwrap();
        let expected_left =
            direct_convolution(&clips[0].samples, &trim_final(&clips[1].samples, 2));
        let expected_right =
            direct_convolution(&trim_start(&clips[0].samples, 2), &clips[1].samples);
        for (actual, expected) in actual.left.iter().zip(expected_left) {
            assert!((actual - expected).abs() < 1.0e-5, "{actual} != {expected}");
        }
        for (actual, expected) in actual.right.iter().zip(expected_right) {
            assert!((actual - expected).abs() < 1.0e-5, "{actual} != {expected}");
        }
    }

    #[test]
    fn complementary_trims_use_faded_cuts() {
        assert_eq!(trim_final(&[1.0, 2.0, 3.0, 4.0], 2), [1.0, 0.0]);
        assert_eq!(trim_start(&[1.0, 2.0, 3.0, 4.0], 2), [0.0, 4.0]);
        assert_eq!(CUT_FADE_FRAMES, 960);
    }

    #[test]
    fn both_channels_have_the_same_trimmed_linear_length() {
        let clips = vec![
            AudioClip {
                id: "a".into(),
                samples: vec![1.0; 10],
            },
            AudioClip {
                id: "b".into(),
                samples: vec![1.0; 20],
            },
        ];
        let job = make_jobs(&clips)
            .into_iter()
            .find(|job| job.left == 0 && job.right == 1)
            .unwrap();
        assert_eq!(job.trim_frames, 5);
        assert_eq!(job.output_frames, 24);
        assert_eq!(10 + (20 - 5) - 1, (10 - 5) + 20 - 1);
    }

    #[test]
    fn complementary_trims_make_a_self_pair_stereo() {
        let clips = vec![AudioClip {
            id: "self".into(),
            samples: vec![0.2, -0.4, 0.8, 0.1, -0.3, 0.7],
        }];
        let job = make_jobs(&clips).remove(0);
        let group = prepare_group(job.fft_len, vec![job.clone()], &clips).unwrap();
        let output = convolve_stereo_spectra(&group, &job, &clips).unwrap();

        assert!(
            output
                .left
                .iter()
                .zip(&output.right)
                .any(|(&left, &right)| (left - right).abs() > 1.0e-5)
        );
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
