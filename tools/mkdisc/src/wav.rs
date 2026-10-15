//! Just enough RIFF/WAVE to feed the level-meter analyser: 16-bit PCM, mono
//! or stereo, any sample rate.

/// A decoded WAV as interleaved stereo little-endian 16-bit PCM bytes.
#[derive(Debug)]
pub struct Wav {
    /// Samples per second, per channel.
    pub rate: u32,
    /// Four bytes per frame, left then right. A mono file is doubled.
    pub stereo_pcm: Vec<u8>,
}

fn le16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Parse a WAV file's bytes.
pub fn parse(bytes: &[u8]) -> Result<Wav, String> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err("not a RIFF/WAVE file".into());
    }
    let mut at = 12;
    let mut format: Option<(u16, u16, u32, u16)> = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = le32(bytes, at + 4).ok_or("truncated chunk header")? as usize;
        let body = at + 8;
        if id == b"fmt " {
            format = Some((
                le16(bytes, body).ok_or("short fmt chunk")?,
                le16(bytes, body + 2).ok_or("short fmt chunk")?,
                le32(bytes, body + 4).ok_or("short fmt chunk")?,
                le16(bytes, body + 14).ok_or("short fmt chunk")?,
            ));
        } else if id == b"data" {
            let (tag, channels, rate, bits) = format.ok_or("data chunk before fmt chunk")?;
            // 1 is PCM; 0xFFFE is the extensible form, which carries PCM too.
            if tag != 1 && tag != 0xFFFE {
                return Err(format!("format tag {tag} is not PCM"));
            }
            if bits != 16 {
                return Err(format!("{bits}-bit samples; only 16-bit PCM is supported"));
            }
            let end = (body + size).min(bytes.len());
            let data = &bytes[body..end];
            let stereo_pcm = match channels {
                2 => data[..data.len() / 4 * 4].to_vec(),
                1 => data
                    .chunks_exact(2)
                    .flat_map(|s| [s[0], s[1], s[0], s[1]])
                    .collect(),
                n => return Err(format!("{n} channels; only mono and stereo are supported")),
            };
            return Ok(Wav { rate, stereo_pcm });
        }
        // Chunks are word aligned.
        at = body + size + (size & 1);
    }
    Err("no data chunk".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(channels: u16, rate: u32, samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2 * channels as u32).to_le_bytes());
        out.extend_from_slice(&(2 * channels).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn stereo_passes_through_at_its_own_rate() {
        let parsed = parse(&wav(2, 37_800, &[1, -2, 3, -4])).expect("valid");
        assert_eq!(parsed.rate, 37_800);
        assert_eq!(parsed.stereo_pcm.len(), 8);
        assert_eq!(&parsed.stereo_pcm[..4], &[1, 0, 0xFE, 0xFF]);
    }

    #[test]
    fn mono_is_doubled_into_both_channels() {
        let parsed = parse(&wav(1, 44_100, &[7, 9])).expect("valid");
        assert_eq!(parsed.stereo_pcm, vec![7, 0, 7, 0, 9, 0, 9, 0]);
    }

    #[test]
    fn something_else_is_an_error() {
        assert!(parse(b"not a wav at all").is_err());
        let mut eight_bit = wav(2, 8000, &[0, 0]);
        eight_bit[34] = 8;
        assert!(parse(&eight_bit).unwrap_err().contains("16-bit"));
    }
}
