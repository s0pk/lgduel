//! Hit sound: synthesised at startup (original, no sample files), mixed on the
//! audio thread. Missing audio hardware is not an error: the game runs silent.

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

/// A sound being played: position in the sample, gain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Voice {
    pub pos: usize,
    pub gain: f32,
}

/// Mix active voices of `sample` into an interleaved buffer with `channels`
/// channels; finished voices are removed. Output is clamped to [-1, 1].
pub fn mix(voices: &mut Vec<Voice>, sample: &[f32], out: &mut [f32], channels: usize) {
    out.fill(0.0);
    let frames = out.len() / channels.max(1);
    for v in voices.iter_mut() {
        for f in 0..frames {
            let Some(s) = sample.get(v.pos + f) else {
                break;
            };
            for c in 0..channels {
                out[f * channels + c] += s * v.gain;
            }
        }
        v.pos += frames;
    }
    voices.retain(|v| v.pos < sample.len());
    for s in out.iter_mut() {
        *s = s.clamp(-1.0, 1.0);
    }
}

/// Handle to the audio output. Dropping it stops the stream.
pub struct Audio {
    tx: Option<Sender<f32>>,
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

    pub fn open(volume: f32) -> Self {
        match Self::try_open(volume) {
            Ok(a) => a,
            Err(e) => Self::silent(&e),
        }
    }

    fn try_open(volume: f32) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("no output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let config = supported.config();
        let channels = config.channels as usize;
        let rate = config.sample_rate;
        let sample = synth_hit(rate);
        let (tx, rx) = channel::<f32>();
        let err = |e| eprintln!("audio stream error: {e}");
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream::<f32, _, _>(
                config,
                mixer(rx, sample, channels, |s| s),
                err,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream::<i16, _, _>(
                config,
                mixer(rx, sample, channels, |s| (s * i16::MAX as f32) as i16),
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
        if let Some(tx) = &self.tx
            && self.volume > 0.0
        {
            let _ = tx.send(self.volume);
        }
    }

    pub fn is_live(&self) -> bool {
        self.tx.is_some()
    }
}

/// Audio-thread callback: take new hits from the channel, mix, convert.
fn mixer<T: Copy + Default>(
    rx: Receiver<f32>,
    sample: Vec<f32>,
    channels: usize,
    convert: impl Fn(f32) -> T + Send + 'static,
) -> impl FnMut(&mut [T], &cpal::OutputCallbackInfo) + Send + 'static {
    let mut voices: Vec<Voice> = Vec::new();
    let mut buf: Vec<f32> = Vec::new();
    move |out: &mut [T], _| {
        while let Ok(gain) = rx.try_recv() {
            voices.push(Voice { pos: 0, gain });
        }
        buf.resize(out.len(), 0.0);
        mix(&mut voices, &sample, &mut buf, channels);
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

    #[test]
    fn mixing_overlaps_and_retires_voices() {
        let sample = vec![0.5f32; 10];
        let mut voices = vec![Voice { pos: 0, gain: 1.0 }, Voice { pos: 6, gain: 1.0 }];
        let mut out = vec![0.0f32; 8]; // 4 frames stereo
        mix(&mut voices, &sample, &mut out, 2);
        assert_eq!(out, vec![1.0; 8]); // both voices overlap, summed (and clamped at 1)
        assert_eq!(voices, vec![Voice { pos: 4, gain: 1.0 }]); // second ran out after 4 frames
        mix(&mut voices, &sample, &mut out, 2);
        assert_eq!(&out[..], &[0.5; 8]);
        mix(&mut voices, &sample, &mut out, 2);
        assert_eq!(&out[..4], &[0.5; 4]);
        assert_eq!(&out[4..], &[0.0; 4]);
        assert!(voices.is_empty());
    }

    #[test]
    fn silent_handle_is_noop() {
        let a = Audio::silent("test");
        a.play_hit();
        assert!(!a.is_live());
    }
}
