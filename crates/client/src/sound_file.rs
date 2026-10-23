//! Decode a sound file (Ogg Vorbis or WAV) into short mono samples, for custom hit
//! and kill sounds. The input is hostile: it is size-capped, every offset is checked
//! and nothing panics. Only the first `MAX_SECS` seconds are kept.

use std::path::Path;

/// Largest file we read.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Longest sound we keep, in seconds.
pub const MAX_SECS: f32 = 2.0;
/// Fade-out applied when a longer sound is cut.
const FADE_SECS: f32 = 0.02;

/// Mono samples in -1..1 at `rate` Hz.
#[derive(Debug, Clone, PartialEq)]
pub struct Pcm {
    pub rate: u32,
    pub samples: Vec<f32>,
}

pub fn decode(bytes: &[u8]) -> Result<Pcm, String> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!(
            "file too large ({} MB, at most {} MB)",
            bytes.len() >> 20,
            MAX_BYTES >> 20
        ));
    }
    let (rate, channels, interleaved, cut) = if bytes.starts_with(b"OggS") {
        vorbis(bytes)?
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        wav(bytes)?
    } else {
        return Err("not an Ogg Vorbis or WAV file".into());
    };
    finish(rate, channels, &interleaved, cut)
}

/// Mix down to mono, clean up, fade a cut sound, reject silence.
fn finish(rate: u32, channels: usize, interleaved: &[f32], cut: bool) -> Result<Pcm, String> {
    let max = (rate as f32 * MAX_SECS) as usize;
    let mut samples: Vec<f32> = interleaved
        .chunks_exact(channels)
        .take(max)
        .map(|f| {
            let s = f.iter().sum::<f32>() / channels as f32;
            if s.is_finite() {
                s.clamp(-1.0, 1.0)
            } else {
                0.0
            }
        })
        .collect();
    let cut = cut || interleaved.len() / channels > max;
    if cut {
        let n = ((rate as f32 * FADE_SECS) as usize).clamp(1, samples.len());
        let start = samples.len() - n;
        for (i, s) in samples[start..].iter_mut().enumerate() {
            *s *= 1.0 - (i + 1) as f32 / n as f32;
        }
    }
    if samples.is_empty() {
        return Err("sound has no samples".into());
    }
    if samples.iter().fold(0.0f32, |m, s| m.max(s.abs())) < 1e-4 {
        return Err("sound is silent".into());
    }
    Ok(Pcm { rate, samples })
}

fn check_format(rate: u32, channels: usize) -> Result<(), String> {
    if !(1..=8).contains(&channels) {
        return Err(format!("{channels} channels (supported: 1 to 8)"));
    }
    if !(8000..=192_000).contains(&rate) {
        return Err(format!("sample rate {rate} Hz (supported: 8000 to 192000)"));
    }
    Ok(())
}

/// `(rate, channels, interleaved samples, was cut)`
type Decoded = (u32, usize, Vec<f32>, bool);

fn vorbis(bytes: &[u8]) -> Result<Decoded, String> {
    use lewton::inside_ogg::OggStreamReader;
    let mut r = OggStreamReader::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("not a valid Ogg Vorbis file ({e})"))?;
    let rate = r.ident_hdr.audio_sample_rate;
    let channels = r.ident_hdr.audio_channels as usize;
    check_format(rate, channels)?;
    let max = (rate as f32 * MAX_SECS) as usize * channels;
    let mut out: Vec<f32> = Vec::new();
    let mut cut = false;
    loop {
        match r.read_dec_packet_itl() {
            Ok(Some(p)) => {
                out.extend(p.iter().map(|&s| s as f32 / 32768.0));
                if out.len() > max {
                    cut = true;
                    break;
                }
            }
            Ok(None) => break,
            // Keep what decoded before the damage; fail only if nothing did.
            Err(e) if out.is_empty() => return Err(format!("Ogg Vorbis decode error ({e})")),
            Err(_) => break,
        }
    }
    Ok((rate, channels, out, cut))
}

fn le16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        b.get(o..o.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn le32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(o..o.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn tag_name(tag: u16) -> &'static str {
    match tag {
        0x0002 => "MS ADPCM",
        0x0011 => "IMA ADPCM",
        0x0050 | 0x0055 => "MP3",
        _ => "unknown",
    }
}

fn wav(b: &[u8]) -> Result<Decoded, String> {
    let mut pos = 12usize;
    let mut fmt: Option<(u16, usize, u32, usize)> = None; // tag, channels, rate, bits
    let mut data: Option<&[u8]> = None;
    while let (Some(id), Some(size)) = (b.get(pos..pos + 4), le32(b, pos + 4)) {
        let start = pos + 8;
        let size = size as usize;
        let end = start.saturating_add(size).min(b.len());
        let body = &b[start.min(b.len())..end];
        match id {
            b"fmt " => {
                if body.len() < 16 || end - start < size {
                    return Err("damaged WAV (fmt chunk)".into());
                }
                let mut tag = le16(body, 0).unwrap_or(0);
                if tag == 0xFFFE {
                    // WAVE_FORMAT_EXTENSIBLE: the real tag is the start of the sub-format GUID.
                    tag = le16(body, 24).ok_or("damaged WAV (extensible fmt chunk)")?;
                }
                fmt = Some((
                    tag,
                    le16(body, 2).unwrap_or(0) as usize,
                    le32(body, 4).unwrap_or(0),
                    le16(body, 14).unwrap_or(0) as usize,
                ));
            }
            b"data" => data = Some(body),
            _ => {}
        }
        if data.is_some() && fmt.is_some() {
            break;
        }
        // Chunks are padded to an even size.
        pos = match start
            .checked_add(size)
            .and_then(|p| p.checked_add(size & 1))
        {
            Some(p) => p,
            None => break,
        };
    }
    let (tag, channels, rate, bits) = fmt.ok_or("damaged WAV (no fmt chunk)")?;
    let data = data.ok_or("damaged WAV (no data chunk)")?;
    if !matches!((tag, bits), (1, 8 | 16 | 24 | 32) | (3, 32)) {
        return Err(if tag == 1 || tag == 3 {
            format!("unsupported WAV encoding ({bits}-bit); re-save it as 16-bit PCM")
        } else {
            format!(
                "unsupported WAV encoding (tag 0x{tag:04x} = {}); re-save it as 16-bit PCM",
                tag_name(tag)
            )
        });
    }
    check_format(rate, channels)?;
    let width = bits / 8;
    let take = (rate as f32 * MAX_SECS) as usize * channels;
    let all = data.len() / width;
    let samples = data
        .chunks_exact(width)
        .take(take + channels)
        .map(|c| match (tag, bits) {
            (1, 8) => (c[0] as f32 - 128.0) / 128.0,
            (1, 16) => i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0,
            (1, 24) => (i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8) as f32 / 8_388_608.0,
            (1, _) => i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / 2_147_483_648.0,
            _ => f32::from_le_bytes([c[0], c[1], c[2], c[3]]),
        })
        .collect();
    Ok((rate, channels, samples, all > take + channels))
}

/// Linear-interpolation resampling (good enough for short effects).
pub fn resample(p: &Pcm, to_rate: u32) -> Vec<f32> {
    if p.rate == to_rate || p.samples.is_empty() || to_rate == 0 {
        return p.samples.clone();
    }
    let ratio = p.rate as f64 / to_rate as f64;
    let n = (p.samples.len() as f64 / ratio).round() as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * ratio;
            let k = x as usize;
            let a = p.samples[k.min(p.samples.len() - 1)];
            let b = p.samples[(k + 1).min(p.samples.len() - 1)];
            a + (b - a) * (x - k as f64) as f32
        })
        .collect()
}

/// Read and decode a file; errors start with the file name.
pub fn load(path: &Path) -> Result<Pcm, String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let fail = |e: String| format!("{name}: {e}");
    let len = std::fs::metadata(path)
        .map_err(|e| fail(e.to_string()))?
        .len();
    if len > MAX_BYTES {
        return Err(fail(format!(
            "file too large ({} MB, at most {} MB)",
            len >> 20,
            MAX_BYTES >> 20
        )));
    }
    let bytes = std::fs::read(path).map_err(|e| fail(e.to_string()))?;
    decode(&bytes).map_err(fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOUNDS: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/testdata/kovaaks/FPSAimTrainer/sounds"
    );

    fn fixture(name: &str) -> Result<Pcm, String> {
        load(&Path::new(SOUNDS).join(name))
    }

    fn peak(p: &Pcm) -> f32 {
        p.samples.iter().fold(0.0, |m, s| m.max(s.abs()))
    }

    fn sane(p: &Pcm) {
        assert!(p.samples.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    }

    /// A WAV file: `extensible` writes a WAVE_FORMAT_EXTENSIBLE header with `tag` as sub-format.
    fn wav_bytes(
        tag: u16,
        bits: u16,
        channels: u16,
        rate: u32,
        data: &[u8],
        extensible: bool,
    ) -> Vec<u8> {
        let mut fmt = Vec::new();
        let hdr_tag = if extensible { 0xFFFE } else { tag };
        fmt.extend(hdr_tag.to_le_bytes());
        fmt.extend(channels.to_le_bytes());
        fmt.extend(rate.to_le_bytes());
        fmt.extend((rate * channels as u32 * bits as u32 / 8).to_le_bytes());
        fmt.extend((channels * bits / 8).to_le_bytes());
        fmt.extend(bits.to_le_bytes());
        if extensible {
            fmt.extend(22u16.to_le_bytes());
            fmt.extend(bits.to_le_bytes());
            fmt.extend(0u32.to_le_bytes());
            fmt.extend(tag.to_le_bytes());
            fmt.extend([0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71]);
        }
        let mut out = b"RIFF".to_vec();
        out.extend(((4 + 8 + fmt.len() + 8 + data.len()) as u32).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend((fmt.len() as u32).to_le_bytes());
        out.extend(&fmt);
        out.extend(b"data");
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }

    fn pcm16(n: usize, rate: u32) -> Vec<u8> {
        (0..n)
            .flat_map(|i| {
                let t = i as f32 / rate as f32;
                (((t * 880.0 * std::f32::consts::TAU).sin() * 20000.0) as i16).to_le_bytes()
            })
            .collect()
    }

    #[test]
    fn fixture_ogg_mono() {
        let p = fixture("lgduel_beep.ogg").unwrap();
        assert_eq!(p.rate, 44100);
        assert!((0.10..0.14).contains(&(p.samples.len() as f32 / 44100.0)));
        assert!(peak(&p) > 0.1);
        sane(&p);
    }

    #[test]
    fn fixture_wav_stereo_downmixed() {
        let p = fixture("lgduel_boop.wav").unwrap();
        assert_eq!(p.rate, 48000);
        assert!((0.08..0.12).contains(&(p.samples.len() as f32 / 48000.0)));
        assert!(peak(&p) > 0.05); // the fixture is quiet (0.088)
        sane(&p);
    }

    #[test]
    fn fixture_88k_resamples() {
        let p = fixture("lgduel_88k.ogg").unwrap();
        assert_eq!(p.rate, 88200);
        let r = resample(&p, 48000);
        let want = p.samples.len() as f64 * 48000.0 / 88200.0;
        assert!(
            (r.len() as f64 - want).abs() <= 2.0,
            "{} vs {want}",
            r.len()
        );
        assert_eq!(resample(&p, 88200), p.samples);
        sane(&p);
    }

    #[test]
    fn adpcm_and_broken_are_refused() {
        let e = fixture("lgduel_adpcm.wav").unwrap_err();
        assert!(
            e.contains(
                "unsupported WAV encoding (tag 0x0011 = IMA ADPCM); re-save it as 16-bit PCM"
            ),
            "{e}"
        );
        assert!(e.starts_with("lgduel_adpcm.wav: "), "{e}");
        let e = fixture("broken.ogg").unwrap_err();
        assert!(e.starts_with("broken.ogg: "), "{e}");
        assert!(fixture("missing.ogg").is_err());
    }

    #[test]
    fn wav_bit_depths_and_floats() {
        let ramp = |f: &dyn Fn(f32) -> Vec<u8>| -> Vec<u8> {
            (0..2400)
                .flat_map(|i| f(if i % 2 == 0 { 0.5 } else { -0.5 }))
                .collect()
        };
        let cases: Vec<Vec<u8>> = vec![
            wav_bytes(
                1,
                8,
                1,
                48000,
                &ramp(&|v| vec![(v * 127.0 + 128.0) as u8]),
                false,
            ),
            wav_bytes(
                1,
                16,
                1,
                48000,
                &ramp(&|v| ((v * 32767.0) as i16).to_le_bytes().to_vec()),
                false,
            ),
            wav_bytes(
                1,
                24,
                1,
                48000,
                &ramp(&|v| ((v * 8_388_607.0) as i32).to_le_bytes()[..3].to_vec()),
                false,
            ),
            wav_bytes(
                1,
                32,
                1,
                48000,
                &ramp(&|v| ((v * 2_147_483_000.0) as i32).to_le_bytes().to_vec()),
                false,
            ),
            wav_bytes(3, 32, 1, 48000, &ramp(&|v| v.to_le_bytes().to_vec()), false),
            wav_bytes(3, 32, 1, 48000, &ramp(&|v| v.to_le_bytes().to_vec()), true),
            wav_bytes(
                1,
                16,
                1,
                48000,
                &ramp(&|v| ((v * 32767.0) as i16).to_le_bytes().to_vec()),
                true,
            ),
        ];
        for (i, c) in cases.iter().enumerate() {
            let p = decode(c).unwrap_or_else(|e| panic!("case {i}: {e}"));
            assert_eq!(p.samples.len(), 2400, "case {i}");
            assert!((peak(&p) - 0.5).abs() < 0.02, "case {i}: {}", peak(&p));
            sane(&p);
        }
    }

    #[test]
    fn wav_damage_never_panics() {
        let good = wav_bytes(1, 16, 1, 48000, &pcm16(4800, 48000), false);
        // Truncated data chunk: what is there is used.
        let p = decode(&good[..good.len() - 1000]).unwrap();
        assert_eq!(p.samples.len(), (4800 * 2 - 1000) / 2);
        // A chunk size bigger than the file.
        let mut big = good.clone();
        big[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&big).is_ok());
        // A huge unknown chunk before the data, and an odd-sized one (padded).
        let mut odd = b"RIFF\0\0\0\0WAVE".to_vec();
        odd.extend(b"junk\x03\0\0\0abc\0");
        odd.extend(&good[12..]);
        assert!(decode(&odd).is_ok());
        let mut huge = b"RIFF\0\0\0\0WAVEjunk".to_vec();
        huge.extend(u32::MAX.to_le_bytes());
        assert!(decode(&huge).is_err());
        // Every truncation of the header and every single-byte garbage header.
        for n in 0..60 {
            let _ = decode(&good[..n.min(good.len())]);
        }
        for i in 0..60 {
            let mut g = good.clone();
            g[i] ^= 0xFF;
            let _ = decode(&g);
        }
        assert!(decode(&[]).is_err());
        assert!(decode(b"RIFF").is_err());
        assert_eq!(
            decode(b"hello world, not audio").unwrap_err(),
            "not an Ogg Vorbis or WAV file"
        );
    }

    #[test]
    fn bad_formats_are_refused() {
        assert!(decode(&wav_bytes(1, 16, 0, 48000, &pcm16(100, 48000), false)).is_err());
        assert!(decode(&wav_bytes(1, 16, 9, 48000, &pcm16(100, 48000), false)).is_err());
        assert!(decode(&wav_bytes(1, 16, 1, 100, &pcm16(100, 100), false)).is_err());
        assert!(decode(&wav_bytes(1, 12, 1, 48000, &pcm16(100, 48000), false)).is_err());
        assert!(decode(&wav_bytes(1, 16, 1, 48000, &[], false)).is_err());
    }

    #[test]
    fn long_sound_is_cut_and_faded() {
        let rate = 8000;
        let loud = [0x00u8, 0x40].repeat(rate as usize * 3); // 3 s of a constant 0.5
        let p = decode(&wav_bytes(1, 16, 1, rate, &loud, false)).unwrap();
        assert_eq!(p.samples.len(), rate as usize * 2);
        assert!(p.samples[0] > 0.49 && p.samples[1000] > 0.49);
        assert!(*p.samples.last().unwrap() < 0.01);
        let n = (rate as f32 * FADE_SECS) as usize;
        assert!((p.samples[p.samples.len() - n - 1] - 0.5).abs() < 0.01);
        // A short sound is left alone.
        let short = decode(&wav_bytes(1, 16, 1, rate, &loud[..4000], false)).unwrap();
        assert_eq!(short.samples.len(), 2000);
        assert!((short.samples[1999] - 0.5).abs() < 0.001);
    }

    #[test]
    fn silence_is_refused() {
        let e = decode(&wav_bytes(1, 16, 1, 48000, &vec![0; 4800], false)).unwrap_err();
        assert_eq!(e, "sound is silent");
    }

    #[test]
    fn size_cap() {
        let mut big = wav_bytes(1, 16, 1, 48000, &pcm16(100, 48000), false);
        big.resize(MAX_BYTES as usize + 1, 0);
        assert!(decode(&big).unwrap_err().contains("too large"));
    }

    #[test]
    fn stereo_average() {
        let mut d = Vec::new();
        for _ in 0..2400 {
            d.extend(16384i16.to_le_bytes());
            d.extend((-16384i16).to_le_bytes());
        }
        let e = decode(&wav_bytes(1, 16, 2, 48000, &d, false)).unwrap_err();
        assert_eq!(e, "sound is silent"); // L and R cancel
        let mut d = Vec::new();
        for _ in 0..2400 {
            d.extend(16384i16.to_le_bytes());
            d.extend(0i16.to_le_bytes());
        }
        let p = decode(&wav_bytes(1, 16, 2, 48000, &d, false)).unwrap();
        assert!((p.samples[0] - 0.25).abs() < 0.001);
    }

    /// Manual check on a PC with KovaaK's: `cargo test -p lgduel-client real_install -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_install_sounds_decode() {
        let inst = crate::kovaaks::find(None).expect("a KovaaK's install");
        let (mut ok, mut bad) = (0, Vec::new());
        for e in std::fs::read_dir(inst.sounds_dir()).unwrap().flatten() {
            match load(&e.path()) {
                Ok(p) => {
                    sane(&p);
                    ok += 1;
                }
                Err(e) => bad.push(e),
            }
        }
        println!("decoded {ok}, refused {}:", bad.len());
        for b in &bad {
            println!("  {b}");
        }
        assert!(ok > 0);
    }
}
