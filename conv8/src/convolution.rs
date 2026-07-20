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

#[derive(Clone, Copy, Debug)]
pub struct ToneCalibration {
    pub unscaled_db_relative: f32,
    pub gain_db: f32,
    pub scaled_db_relative: f32,
}

pub fn make_jobs(
    clips: &[AudioClip],
    short_indices: &[usize],
    long_indices: &[usize],
) -> Vec<PairJob> {
    let mut jobs = Vec::with_capacity(short_indices.len() * long_indices.len());
    for &left in short_indices {
        for &right in long_indices {
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
    preprocessed_track_1: Option<&[f32]>,
    preprocessed_track_2: Option<&[f32]>,
) -> Result<StereoAudio> {
    let track_1_samples = preprocessed_track_1.unwrap_or(&clips[job.left].samples);
    let track_2_samples = preprocessed_track_2.unwrap_or(&clips[job.right].samples);
    if track_1_samples.len() != clips[job.left].samples.len() {
        anyhow::bail!("preprocessed track 1 changed length");
    }
    if track_2_samples.len() != clips[job.right].samples.len() {
        anyhow::bail!("preprocessed track 2 changed length");
    }
    let owned_track_1 = preprocessed_track_1
        .map(|samples| forward_transform(&*group.forward, group.fft_len, samples))
        .transpose()
        .with_context(|| format!("preprocessed FFT for {}", clips[job.left].id))?;
    let track_1 = owned_track_1
        .as_deref()
        .unwrap_or(&group.spectra[&job.left]);
    let owned_track_2 = preprocessed_track_2
        .map(|samples| forward_transform(&*group.forward, group.fft_len, samples))
        .transpose()
        .with_context(|| format!("preprocessed FFT for {}", clips[job.right].id))?;
    let track_2 = owned_track_2
        .as_deref()
        .unwrap_or(&group.spectra[&job.right]);

    let shortened_track_2 = trim_final(track_2_samples, job.trim_frames);
    let shortened_track_2_spectrum =
        forward_transform(&*group.forward, group.fft_len, &shortened_track_2)
            .with_context(|| format!("shortened FFT for {}", clips[job.right].id))?;
    let left_channel = inverse_channel(group, job, track_1, &shortened_track_2_spectrum)?;

    let shortened_track_1 = trim_start(track_1_samples, job.trim_frames);
    let shortened_track_1_spectrum =
        forward_transform(&*group.forward, group.fft_len, &shortened_track_1)
            .with_context(|| format!("shortened FFT for {}", clips[job.left].id))?;
    let right_channel = inverse_channel(group, job, &shortened_track_1_spectrum, track_2)?;

    Ok(StereoAudio {
        left: left_channel,
        right: right_channel,
    })
}

pub fn convolve_stereo_with_tone(
    group: &SpectralGroup,
    job: &PairJob,
    clips: &[AudioClip],
    tone_track_1: Option<&[f32]>,
    tone_track_2: Option<&[f32]>,
    target_db_relative: f32,
) -> Result<(StereoAudio, ToneCalibration)> {
    if tone_track_1.is_some() == tone_track_2.is_some() {
        anyhow::bail!("exactly one tone stem must be provided");
    }
    if !target_db_relative.is_finite() {
        anyhow::bail!("tone target must be finite");
    }

    let dry = convolve_stereo_spectra(group, job, clips, None, None)?;
    let tone = convolve_stereo_spectra(group, job, clips, tone_track_1, tone_track_2)?;
    let dry_rms = stereo_rms(&dry);
    let tone_rms = stereo_rms(&tone);
    if dry_rms < 1.0e-12 || tone_rms < 1.0e-12 {
        anyhow::bail!("cannot calibrate silent convolution component");
    }

    let unscaled_db_relative = 20.0 * (tone_rms / dry_rms).log10();
    let gain_db = target_db_relative - unscaled_db_relative;
    let gain = 10.0_f32.powf(gain_db / 20.0);
    if !gain.is_finite() {
        anyhow::bail!("tone calibration produced a non-finite gain");
    }
    let scaled_db_relative = 20.0 * (tone_rms * gain / dry_rms).log10();
    let left = dry
        .left
        .iter()
        .zip(&tone.left)
        .map(|(&dry, &tone)| tone.mul_add(gain, dry))
        .collect();
    let right = dry
        .right
        .iter()
        .zip(&tone.right)
        .map(|(&dry, &tone)| tone.mul_add(gain, dry))
        .collect();
    Ok((
        StereoAudio { left, right },
        ToneCalibration {
            unscaled_db_relative,
            gain_db,
            scaled_db_relative,
        },
    ))
}

fn stereo_rms(audio: &StereoAudio) -> f32 {
    let sum_squares = audio
        .left
        .iter()
        .chain(&audio.right)
        .map(|&sample| f64::from(sample) * f64::from(sample))
        .sum::<f64>();
    (sum_squares / (audio.left.len().max(1) * 2) as f64).sqrt() as f32
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
        let job = make_jobs(&clips, &[0], &[1])
            .into_iter()
            .find(|job| job.left == 0 && job.right == 1)
            .unwrap();
        assert_eq!(job.trim_frames, 2);
        assert_eq!(job.output_frames, 4);
        let group = prepare_group(job.fft_len, vec![job.clone()], &clips).unwrap();
        let actual = convolve_stereo_spectra(&group, &job, &clips, None, None).unwrap();
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
    fn preprocessed_long_clip_is_used_in_both_channels() {
        let clips = vec![
            AudioClip {
                id: "short".into(),
                samples: vec![0.25, -0.5, 1.0, 0.125],
            },
            AudioClip {
                id: "long".into(),
                samples: vec![0.3, 0.2, -0.1],
            },
        ];
        let processed_long = vec![-0.7, 0.4, 0.9];
        let job = make_jobs(&clips, &[0], &[1]).remove(0);
        let group = prepare_group(job.fft_len, vec![job.clone()], &clips).unwrap();
        let actual =
            convolve_stereo_spectra(&group, &job, &clips, None, Some(&processed_long)).unwrap();
        let expected_left = direct_convolution(
            &clips[0].samples,
            &trim_final(&processed_long, job.trim_frames),
        );
        let expected_right = direct_convolution(
            &trim_start(&clips[0].samples, job.trim_frames),
            &processed_long,
        );

        for (actual, expected) in actual.left.iter().zip(expected_left) {
            assert!((actual - expected).abs() < 1.0e-5, "{actual} != {expected}");
        }
        for (actual, expected) in actual.right.iter().zip(expected_right) {
            assert!((actual - expected).abs() < 1.0e-5, "{actual} != {expected}");
        }
    }

    #[test]
    fn tone_convolution_is_calibrated_relative_to_dry_convolution() {
        let clips = vec![
            AudioClip {
                id: "short".into(),
                samples: vec![0.25, -0.5, 1.0, 0.125],
            },
            AudioClip {
                id: "long".into(),
                samples: vec![0.3, 0.2, -0.1],
            },
        ];
        let tone = [0.0, 0.2, -0.1];
        let job = make_jobs(&clips, &[0], &[1]).remove(0);
        let group = prepare_group(job.fft_len, vec![job.clone()], &clips).unwrap();
        let (mixed, calibration) =
            convolve_stereo_with_tone(&group, &job, &clips, None, Some(&tone), -1.5).unwrap();
        let dry = convolve_stereo_spectra(&group, &job, &clips, None, None).unwrap();
        let effect = StereoAudio {
            left: mixed
                .left
                .iter()
                .zip(&dry.left)
                .map(|(&mixed, &dry)| mixed - dry)
                .collect(),
            right: mixed
                .right
                .iter()
                .zip(&dry.right)
                .map(|(&mixed, &dry)| mixed - dry)
                .collect(),
        };
        let measured_db = 20.0 * (stereo_rms(&effect) / stereo_rms(&dry)).log10();
        assert!((measured_db - (-1.5)).abs() < 1.0e-4);
        assert!((calibration.scaled_db_relative - (-1.5)).abs() < 1.0e-4);
        assert!((calibration.gain_db - (-1.5 - calibration.unscaled_db_relative)).abs() < 1.0e-5);
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
        let job = make_jobs(&clips, &[0], &[1])
            .into_iter()
            .find(|job| job.left == 0 && job.right == 1)
            .unwrap();
        assert_eq!(job.trim_frames, 5);
        assert_eq!(job.output_frames, 24);
        assert_eq!(10 + (20 - 5) - 1, (10 - 5) + 20 - 1);
    }

    #[test]
    fn complementary_trims_make_cross_duration_pairs_stereo() {
        let clips = vec![
            AudioClip {
                id: "short".into(),
                samples: vec![0.2, -0.4, 0.8, 0.1, -0.3, 0.7],
            },
            AudioClip {
                id: "long".into(),
                samples: vec![0.1, 0.3, -0.2, 0.9, 0.2, -0.8, 0.4, 0.6],
            },
        ];
        let job = make_jobs(&clips, &[0], &[1]).remove(0);
        let group = prepare_group(job.fft_len, vec![job.clone()], &clips).unwrap();
        let output = convolve_stereo_spectra(&group, &job, &clips, None, None).unwrap();

        assert!(
            output
                .left
                .iter()
                .zip(&output.right)
                .any(|(&left, &right)| (left - right).abs() > 1.0e-5)
        );
    }

    #[test]
    fn job_count_is_complete_bipartite_matrix() {
        let clips = (0..48)
            .map(|index| AudioClip {
                id: format!("clip_{index}"),
                samples: vec![1.0],
            })
            .collect::<Vec<_>>();
        let short_indices = (0..24).collect::<Vec<_>>();
        let long_indices = (24..48).collect::<Vec<_>>();
        let jobs = make_jobs(&clips, &short_indices, &long_indices);
        assert_eq!(jobs.len(), 576);
        assert!(jobs.iter().all(|job| job.left < 24 && job.right >= 24));
    }
}
