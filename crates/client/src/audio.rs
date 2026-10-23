//! Sounds: all synthesised at startup from sines (original, no sample files),
//! mixed on the audio thread. Missing audio hardware is not an error: the game
//! runs silent.

use crate::sound_file::{self, Pcm};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc::{Receiver, Sender, channel};

/// Short bright "tick": two sines (1.9 kHz + 2.85 kHz) with a 1 ms attack and
/// exponential decay, 45 ms long. Mono, peak-normalised to 0.9.
pub fn synth_hit(sample_rate: u32) -> Vec<f32> {
    let n = (sample_rate as f32 * 0.045) as usize;
    let attack = (sample_rate as f32 * 0.001).max(1.0);
    let mut v: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            let env = (i as f32 / attack).min(1.0) * (-t * 90.0).exp();
            let s = (std::f32::consts::TAU * 1900.0 * t).sin() * 0.6
                + (std::f32::consts::TAU * 2850.0 * t).sin() * 0.4;
            s * env
        })
        .collect();
    let peak = v.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak > 0.0 {
        for s in &mut v {
            *s *= 0.9 / peak;
        }
    }
    v
}

/// The sounds the game can play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    /// Your beam hit (the original sound).
    Hit,
    /// You scored a frag.
    Kill,
    FightWon,
    FightLost,
    UiClick,
    /// Someone challenged you.
    Challenge,
}

impl Sound {
    pub const ALL: [Sound; 6] = [
        Sound::Hit,
        Sound::Kill,
        Sound::FightWon,
        Sound::FightLost,
        Sound::UiClick,
        Sound::Challenge,
    ];

    fn index(self) -> usize {
        self as usize
    }
}

/// Sine notes `(frequency Hz, length ms)` one after another, each with a 1 ms
/// attack and exponential decay `decay` per second; peak-normalised to 0.8.
fn tones(rate: u32, notes: &[(f32, f32)], decay: f32) -> Vec<f32> {
    let mut v = Vec::new();
    for &(freq, ms) in notes {
        let n = (rate as f32 * ms / 1000.0) as usize;
        let attack = (rate as f32 * 0.001).max(1.0);
        v.extend((0..n).map(|i| {
            let t = i as f32 / rate as f32;
            let env = (i as f32 / attack).min(1.0) * (-t * decay).exp();
            (std::f32::consts::TAU * freq * t).sin() * env
        }));
    }
    let peak = v.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak > 0.0 {
        for s in &mut v {
            *s *= 0.8 / peak;
        }
    }
    v
}

/// The samples of `sound` at `rate` Hz (mono).
pub fn synth(sound: Sound, rate: u32) -> Vec<f32> {
    match sound {
        Sound::Hit => synth_hit(rate),
        Sound::Kill => tones(rate, &[(880.0, 60.0), (1320.0, 110.0)], 22.0),
        Sound::FightWon => tones(rate, &[(523.0, 90.0), (659.0, 90.0), (784.0, 160.0)], 14.0),
        Sound::FightLost => tones(rate, &[(392.0, 130.0), (294.0, 220.0)], 12.0),
        Sound::UiClick => tones(rate, &[(1200.0, 14.0)], 120.0),
        Sound::Challenge => tones(rate, &[(988.0, 70.0), (1318.0, 140.0)], 18.0),
    }
}

/// Player-chosen sounds (imported files) that replace the synthesised hit / kill sound.
#[derive(Debug, Clone, Default)]
pub struct CustomSounds {
    pub hit: Option<Pcm>,
    pub kill: Option<Pcm>,
}

impl CustomSounds {
    /// The samples of `sound` at `rate`: the custom file if there is one, else the synth.
    pub fn samples(&self, sound: Sound, rate: u32) -> Vec<f32> {
        let custom = match sound {
            Sound::Hit => self.hit.as_ref(),
            Sound::Kill => self.kill.as_ref(),
            _ => None,
        };
        match custom {
            Some(p) => sound_file::resample(p, rate),
            None => synth(sound, rate),
        }
    }
}

/// Most hit sounds playing at once; a long custom sound would otherwise pile up
/// (the LG hits about 20 times a second).
pub const MAX_HIT_VOICES: usize = 4;

/// Start `v`; if it is a hit sound and `MAX_HIT_VOICES` are already playing, the oldest stops.
pub fn add_voice(voices: &mut Vec<Voice>, v: Voice) {
    if v.sound == Sound::Hit.index() {
        let active = voices.iter().filter(|o| o.sound == v.sound).count();
        if active >= MAX_HIT_VOICES
            && let Some(oldest) = voices.iter().position(|o| o.sound == v.sound)
        {
            voices.remove(oldest);
        }
    }
    voices.push(v);
}

/// A sound being played: which sample, read position (fractional: pitch), speed, gain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Voice {
    pub sound: usize,
    pub pos: f32,
    /// Samples advanced per output frame (1.0 = normal pitch).
    pub step: f32,
    pub gain: f32,
}

/// What the game thread asks the audio thread to play.
#[derive(Debug, Clone, Copy)]
struct Play {
    sound: Sound,
    gain: f32,
    pitch: f32,
}

/// Mix active voices into an interleaved buffer with `channels` channels
/// (`samples[voice.sound]` is each voice's sample); finished voices are
/// removed. Output is clamped to [-1, 1].
pub fn mix(voices: &mut Vec<Voice>, samples: &[Vec<f32>], out: &mut [f32], channels: usize) {
    out.fill(0.0);
    let frames = out.len() / channels.max(1);
    for v in voices.iter_mut() {
        let sample = &samples[v.sound];
        for f in 0..frames {
            let p = v.pos + v.step * f as f32;
            let i = p as usize;
            let Some(a) = sample.get(i) else {
                break;
            };
            let b = sample.get(i + 1).copied().unwrap_or(0.0);
            let s = a + (b - a) * (p - i as f32);
            for c in 0..channels {
                out[f * channels + c] += s * v.gain;
            }
        }
        v.pos += v.step * frames as f32;
    }
    voices.retain(|v| (v.pos as usize) < samples[v.sound].len());
    for s in out.iter_mut() {
        *s = s.clamp(-1.0, 1.0);
    }
}

/// Handle to the audio output. Dropping it stops the stream.
pub struct Audio {
    tx: Option<Sender<Play>>,
    _stream: Option<cpal::Stream>,
    pub volume: f32,
    pub description: String,
}

impl Audio {
    /// Silent handle (sound disabled or no device).
    pub fn silent(reason: &str) -> Self {
        Self {
            tx: None,
            _stream: None,
            volume: 0.0,
            description: format!("silent ({reason})"),
        }
    }

    pub fn open(volume: f32, custom: &CustomSounds) -> Self {
        match Self::try_open(volume, custom) {
            Ok(a) => a,
            Err(e) => Self::silent(&e),
        }
    }

    fn try_open(volume: f32, custom: &CustomSounds) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("no output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let config = supported.config();
        let channels = config.channels as usize;
        let rate = config.sample_rate;
        let samples: Vec<Vec<f32>> = Sound::ALL
            .iter()
            .map(|s| custom.samples(*s, rate))
            .collect();
        let (tx, rx) = channel::<Play>();
        let err = |e| eprintln!("audio stream error: {e}");
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream::<f32, _, _>(
                config,
                mixer(rx, samples, channels, |s| s),
                err,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream::<i16, _, _>(
                config,
                mixer(rx, samples, channels, |s| (s * i16::MAX as f32) as i16),
                err,
                None,
            ),
            other => return Err(format!("unsupported sample format {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        let name = device
            .description()
            .map(|d| d.to_string())
            .unwrap_or_else(|_| "default device".into());
        Ok(Self {
            tx: Some(tx),
            _stream: Some(stream),
            volume,
            description: format!("{name} @ {rate} Hz, {channels} ch"),
        })
    }

    pub fn play_hit(&self) {
        self.play(Sound::Hit, 1.0, 1.0);
    }

    /// Play `sound` at `gain` (times the master volume) and `pitch` (1.0 = normal).
    pub fn play(&self, sound: Sound, gain: f32, pitch: f32) {
        if let Some(tx) = &self.tx
            && self.volume > 0.0
        {
            let _ = tx.send(Play {
                sound,
                gain: self.volume * gain,
                pitch: pitch.clamp(0.5, 2.0),
            });
        }
    }

    pub fn is_live(&self) -> bool {
        self.tx.is_some()
    }
}

/// Audio-thread callback: take new sounds from the channel, mix, convert.
fn mixer<T: Copy + Default>(
    rx: Receiver<Play>,
    samples: Vec<Vec<f32>>,
    channels: usize,
    convert: impl Fn(f32) -> T + Send + 'static,
) -> impl FnMut(&mut [T], &cpal::OutputCallbackInfo) + Send + 'static {
    let mut voices: Vec<Voice> = Vec::new();
    let mut buf: Vec<f32> = Vec::new();
    move |out: &mut [T], _| {
        while let Ok(p) = rx.try_recv() {
            add_voice(
                &mut voices,
                Voice {
                    sound: p.sound.index(),
                    pos: 0.0,
                    step: p.pitch,
                    gain: p.gain,
                },
            );
        }
        buf.resize(out.len(), 0.0);
        mix(&mut voices, &samples, &mut buf, channels);
        for (o, s) in out.iter_mut().zip(&buf) {
            *o = convert(*s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_sound_shape() {
        let s = synth_hit(48_000);
        assert_eq!(s.len(), 2160); // 45 ms
        let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.9).abs() < 1e-5);
        let energy = |r: std::ops::Range<usize>| s[r].iter().map(|v| v * v).sum::<f32>();
        assert!(energy(0..480) > 20.0 * energy(1680..2160), "must decay");
        assert_eq!(s[0], 0.0, "starts silent (no click)");
    }

    fn voice(pos: f32) -> Voice {
        Voice {
            sound: 0,
            pos,
            step: 1.0,
            gain: 1.0,
        }
    }

    #[test]
    fn mixing_overlaps_and_retires_voices() {
        let samples = vec![vec![0.5f32; 10]];
        let mut voices = vec![voice(0.0), voice(6.0)];
        let mut out = vec![0.0f32; 8]; // 4 frames stereo
        mix(&mut voices, &samples, &mut out, 2);
        assert_eq!(out, vec![1.0; 8]); // both voices overlap, summed (and clamped at 1)
        assert_eq!(voices, vec![voice(4.0)]); // second ran out after 4 frames
        mix(&mut voices, &samples, &mut out, 2);
        assert_eq!(&out[..], &[0.5; 8]);
        mix(&mut voices, &samples, &mut out, 2);
        assert_eq!(&out[..4], &[0.5; 4]);
        assert_eq!(&out[4..], &[0.0; 4]);
        assert!(voices.is_empty());
    }

    #[test]
    fn pitch_changes_playback_speed() {
        let ramp: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
        let samples = vec![ramp];
        let mut fast = vec![Voice {
            step: 2.0,
            ..voice(0.0)
        }];
        let mut out = vec![0.0f32; 4];
        mix(&mut fast, &samples, &mut out, 1);
        assert_eq!(
            out,
            vec![0.0, 0.02, 0.04, 0.06],
            "twice as fast: every second sample"
        );
        // Interpolates between samples at fractional steps.
        let mut half = vec![Voice {
            step: 0.5,
            ..voice(0.0)
        }];
        mix(&mut half, &samples, &mut out, 1);
        assert!(
            (out[1] - 0.005).abs() < 1e-6 && (out[2] - 0.01).abs() < 1e-6,
            "{out:?}"
        );
    }

    #[test]
    fn every_sound_is_finite_short_audible_and_deterministic() {
        for s in Sound::ALL {
            let a = synth(s, 48_000);
            assert_eq!(a, synth(s, 48_000), "{s:?}");
            assert!(
                !a.is_empty() && a.len() < 48_000 * 6 / 10,
                "{s:?}: {} samples",
                a.len()
            );
            assert!(a.iter().all(|v| v.is_finite() && v.abs() <= 1.0), "{s:?}");
            let peak = a.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.5, "{s:?} is nearly silent: {peak}");
            assert_eq!(a[0], 0.0, "{s:?} starts silent (no click)");
        }
        // Distinct sounds.
        assert_ne!(synth(Sound::Kill, 48_000), synth(Sound::Challenge, 48_000));
        assert!(synth(Sound::FightWon, 48_000).len() > synth(Sound::UiClick, 48_000).len() * 10);
    }

    #[test]
    fn hit_voices_are_capped_oldest_first() {
        let hit = Sound::Hit.index();
        let mk = |pos: f32, sound: usize| Voice {
            sound,
            pos,
            step: 1.0,
            gain: 1.0,
        };
        let mut v = Vec::new();
        add_voice(&mut v, mk(0.0, Sound::Kill.index()));
        for i in 0..6 {
            add_voice(&mut v, mk(i as f32, hit));
        }
        let hits: Vec<f32> = v.iter().filter(|x| x.sound == hit).map(|x| x.pos).collect();
        assert_eq!(hits, vec![2.0, 3.0, 4.0, 5.0]);
        assert!(
            v.iter().any(|x| x.sound == Sound::Kill.index()),
            "other sounds are not capped"
        );
    }

    #[test]
    fn custom_sounds_replace_only_hit_and_kill() {
        let pcm = Pcm {
            rate: 44_100,
            samples: vec![0.5; 4410],
        };
        let c = CustomSounds {
            hit: Some(pcm),
            kill: None,
        };
        let hit = c.samples(Sound::Hit, 48_000);
        assert!((hit.len() as i64 - 4800).abs() <= 2, "{}", hit.len());
        assert_eq!(c.samples(Sound::Kill, 48_000), synth(Sound::Kill, 48_000));
        assert_eq!(
            c.samples(Sound::UiClick, 48_000),
            synth(Sound::UiClick, 48_000)
        );
        assert_eq!(
            CustomSounds::default().samples(Sound::Hit, 48_000),
            synth(Sound::Hit, 48_000)
        );
    }

    #[test]
    fn silent_handle_is_noop() {
        let a = Audio::silent("test");
        a.play_hit();
        a.play(Sound::Kill, 1.0, 1.5);
        assert!(!a.is_live());
    }
}
