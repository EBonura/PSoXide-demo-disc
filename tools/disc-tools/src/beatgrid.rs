//! Find the beat grid of a song.
//!
//! The menu pulses its visuals on the beat, driven by the XA song clock,
//! which reports milliseconds into the playing song. To turn that into beats
//! the launcher needs a tempo and a phase, and both have to be accurate: a
//! tempo off by 1 BPM drifts several beats out of step over a four-minute
//! track, which reads as broken rather than as an effect.
//!
//! So this measures rather than trusts the tempo the artist quoted. It builds
//! an onset-strength envelope, scores every (tempo, phase) pair on a fine grid
//! by how much onset energy lands on the beats, and prints the winner in the
//! form `mkdisc --menu-beat` wants.
//!
//!     disc-tools beatgrid [--bpm LOW:HIGH] song.wav...
//!
//! Input is a 16-bit PCM WAV at any sample rate (the songs as authored), or a
//! raw 44.1 kHz 16-bit stereo `.cdda` file. The tempo search runs over
//! 160 to 190 BPM unless `--bpm` widens it; a song outside the range gets
//! the nearest edge, so check the printed tempo against what you hear.
//!
//! The arithmetic follows the Python original operation for operation, so the
//! tempo it picks cannot differ on a tie or a rounding: the winner is the
//! first of the highest scores, and the scores are summed in the same order.

use std::fs;

use crate::util::{Error, Result};

/// Rate of the raw `.cdda` input.
const CDDA_RATE: i64 = 44100;
/// Default tempo search, in tenths of a BPM: drum and bass.
const DEFAULT_BPM: (u32, u32) = (1600, 1900);
/// About 6 ms, fine enough to place a beat inside one frame at 60 Hz.
const HOP: usize = 256;

/// Python 3.12+ sums floats with Neumaier compensation, and the original
/// leaned on `sum()`. Doing the same keeps the mean bit-identical.
fn float_sum(values: &[f64]) -> f64 {
    let mut total = 0.0f64;
    let mut compensation = 0.0f64;
    for &x in values {
        let t = total + x;
        if total.abs() >= x.abs() {
            compensation += (total - t) + x;
        } else {
            compensation += (x - t) + total;
        }
        total = t;
    }
    if compensation != 0.0 && compensation.is_finite() {
        total += compensation;
    }
    total
}

/// Half-wave rectified energy difference, one value per HOP samples.
fn onset_envelope(raw: &[u8]) -> Vec<f64> {
    let frames = raw.len() / 4;
    let sample = |n: usize| i16::from_le_bytes([raw[2 * n], raw[2 * n + 1]]) as i64;
    let mut energy = Vec::new();
    let mut start = 0;
    while start + HOP < frames {
        let mut total: i64 = 0;
        for i in start..start + HOP {
            total += sample(2 * i).abs() + sample(2 * i + 1).abs();
        }
        energy.push(total as f64 / (2 * HOP) as f64);
        start += HOP;
    }
    (1..energy.len())
        .map(|i| (energy[i] - energy[i - 1]).max(0.0))
        .collect()
}

/// Total onset strength landing on a grid at `bpm` starting `phase_frames` in.
fn score(envelope: &[f64], rate: f64, bpm: f64, phase_frames: usize) -> f64 {
    let spacing = rate * 60.0 / bpm;
    let mut total = 0.0;
    let mut beat = 0u64;
    loop {
        let at = phase_frames as f64 + beat as f64 * spacing;
        let index = (at + 0.5) as usize;
        if index >= envelope.len() {
            return total;
        }
        // A beat rarely lands exactly on a frame boundary, so take the best of
        // the frame and its neighbours.
        let lo = index.saturating_sub(1);
        let hi = (index + 2).min(envelope.len());
        let mut best = envelope[lo];
        for &value in &envelope[lo + 1..hi] {
            if value > best {
                best = value;
            }
        }
        total += best;
        beat += 1;
    }
}

/// `(milli_bpm, phase_ms)` for one track's PCM.
fn beat_grid(raw: &[u8], sample_rate: i64, bpm_tenths: (u32, u32)) -> Result<(i64, i64)> {
    let mut envelope = onset_envelope(raw);
    ensure!(!envelope.is_empty(), "track is too short to measure");
    let rate = sample_rate as f64 / HOP as f64;
    let mean = float_sum(&envelope) / envelope.len() as f64;
    for e in envelope.iter_mut() {
        *e = (*e - mean).max(0.0);
    }

    // Coarse pass over plausible drum and bass tempos, then refine around the
    // winner. A single fine sweep would be minutes of work for the same answer.
    let mut best: Option<(f64, f64, usize)> = None;
    for tenths in bpm_tenths.0..bpm_tenths.1 {
        let bpm = tenths as f64 / 10.0;
        let spacing = rate * 60.0 / bpm;
        let mut phase = 0;
        while phase <= spacing as usize {
            let value = score(&envelope, rate, bpm, phase);
            if best.is_none_or(|b| value > b.0) {
                best = Some((value, bpm, phase));
            }
            phase += 2;
        }
    }
    let Some((_, coarse_bpm, coarse_phase)) = best else {
        bail!("no tempo candidates")
    };

    let centre = (coarse_bpm * 1000.0) as i64;
    let mut best: Option<(f64, i64, usize)> = None;
    let mut milli = centre - 400;
    while milli < centre + 401 {
        let bpm = milli as f64 / 1000.0;
        for phase in coarse_phase.saturating_sub(4)..coarse_phase + 5 {
            let value = score(&envelope, rate, bpm, phase);
            if best.is_none_or(|b| value > b.0) {
                best = Some((value, milli, phase));
            }
        }
        milli += 10;
    }
    let Some((_, milli_bpm, phase_frames)) = best else {
        bail!("no tempo candidates")
    };
    let phase_ms = (phase_frames as i64 * HOP as i64 * 1000) as f64 / sample_rate as f64;
    Ok((milli_bpm, phase_ms as i64))
}

/// Python's `"%-28s %7.3f BPM, first beat %4d ms   --menu-beat %d:%d"`.
fn format_line(path: &str, milli_bpm: i64, phase_ms: i64) -> String {
    format!(
        "{path:<28} {:7.3} BPM, first beat {phase_ms:4} ms   --menu-beat {milli_bpm}:{phase_ms}",
        milli_bpm as f64 / 1000.0
    )
}

/// Stereo 16-bit PCM bytes and the sample rate of a 16-bit PCM WAV.
fn wav_pcm(bytes: &[u8]) -> Result<(Vec<u8>, i64)> {
    ensure!(
        bytes.get(0..4) == Some(b"RIFF") && bytes.get(8..12) == Some(b"WAVE"),
        "not a RIFF/WAVE file"
    );
    let le16 = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let le32 = |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let mut at = 12;
    let mut format = None;
    while at + 8 <= bytes.len() {
        let size = le32(at + 4) as usize;
        let body = at + 8;
        match &bytes[at..at + 4] {
            b"fmt " => format = Some((le16(body), le16(body + 2), le32(body + 4), le16(body + 14))),
            b"data" => {
                let Some((tag, channels, rate, bits)) = format else {
                    bail!("data chunk before fmt chunk")
                };
                ensure!(tag == 1 || tag == 0xFFFE, "format tag {tag} is not PCM");
                ensure!(bits == 16, "{bits}-bit samples; only 16-bit PCM is supported");
                let data = &bytes[body..(body + size).min(bytes.len())];
                let pcm = match channels {
                    2 => data[..data.len() / 4 * 4].to_vec(),
                    1 => data
                        .chunks_exact(2)
                        .flat_map(|s| [s[0], s[1], s[0], s[1]])
                        .collect(),
                    n => bail!("{n} channels; only mono and stereo are supported"),
                };
                return Ok((pcm, rate as i64));
            }
            _ => {}
        }
        at = body + size + (size & 1);
    }
    bail!("no data chunk")
}

pub fn run(args: &[String]) -> Result<i32> {
    let mut bpm = DEFAULT_BPM;
    let mut paths = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--bpm" {
            let spec = it.next().ok_or_else(|| Error("--bpm takes LOW:HIGH".into()))?;
            let parsed = spec.split_once(':').and_then(|(lo, hi)| {
                Some((lo.parse::<f64>().ok()?, hi.parse::<f64>().ok()?))
            });
            let Some((lo, hi)) = parsed.filter(|(lo, hi)| lo > &0.0 && lo < hi) else {
                bail!("--bpm wants LOW:HIGH in BPM, got {spec:?}")
            };
            bpm = ((lo * 10.0) as u32, (hi * 10.0) as u32);
        } else {
            paths.push(arg);
        }
    }
    if paths.is_empty() {
        println!("usage: disc-tools beatgrid [--bpm LOW:HIGH] <song.wav|file.cdda>...");
        return Ok(1);
    }
    for path in paths {
        let raw = fs::read(path).map_err(|e| Error(format!("{path}: {e}")))?;
        let (pcm, rate) = if path.to_ascii_lowercase().ends_with(".wav") {
            wav_pcm(&raw).map_err(|e| Error(format!("{path}: {e}")))?
        } else {
            (raw, CDDA_RATE)
        };
        let (milli_bpm, phase_ms) = beat_grid(&pcm, rate, bpm)?;
        println!("{}", format_line(path, milli_bpm, phase_ms));
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A click track: one loud frame block every `period` hops over silence.
    fn click_track(bpm: f64, seconds: usize) -> Vec<u8> {
        let frames = CDDA_RATE as usize * seconds;
        let mut raw = vec![0u8; frames * 4];
        let spacing = CDDA_RATE as f64 * 60.0 / bpm;
        let mut at = 0.0;
        while (at as usize) + 600 < frames {
            for i in at as usize..at as usize + 600 {
                raw[4 * i..4 * i + 2].copy_from_slice(&20000i16.to_le_bytes());
                raw[4 * i + 2..4 * i + 4].copy_from_slice(&20000i16.to_le_bytes());
            }
            at += spacing;
        }
        raw
    }

    #[test]
    fn finds_the_tempo_of_a_click_track() {
        let (milli, phase) = beat_grid(&click_track(172.0, 40), CDDA_RATE, DEFAULT_BPM).unwrap();
        assert!((milli - 172_000).abs() <= 300, "got {milli}");
        assert!(phase < 40, "first beat at {phase} ms");
    }

    #[test]
    fn output_line_matches_the_python_format() {
        assert_eq!(
            format_line("audio/knuckle-dust.cdda", 176010, 359),
            "audio/knuckle-dust.cdda      176.010 BPM, first beat  359 ms   --menu-beat 176010:359"
        );
        assert_eq!(
            format_line("a", 1000, 5),
            format!(
                "{:<28}   1.000 BPM, first beat    5 ms   --menu-beat 1000:5",
                "a"
            )
        );
    }

    #[test]
    fn float_sum_is_compensated_like_python() {
        // Plain left-to-right addition of these gives 0.0; Python 3.12+ gives 1.0.
        assert_eq!(float_sum(&[1.0, 1e100, 1.0, -1e100]), 2.0);
        assert_eq!(float_sum(&[0.1; 10]), 1.0);
    }

    #[test]
    fn short_input_is_an_error_not_a_panic() {
        assert!(beat_grid(&[0u8; 1000], CDDA_RATE, DEFAULT_BPM).is_err());
    }
}
