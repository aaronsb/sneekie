//! Square-wave audio, behind the `audio` feature — the PC-speaker voice of the
//! original `SOUND` statement.
//!
//! Each [`Game::snd`](super::Game) call synthesizes a short square wave at the
//! requested frequency and appends it to a [`rodio::Sink`], so tones queue and
//! play in sequence exactly like the queued `SOUND` calls of the GW-BASIC
//! original. We generate raw samples into a [`SamplesBuffer`] rather than
//! implementing the `Source` trait, to stay stable across rodio versions.

use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, OutputStreamHandle, Sink};

const SAMPLE_RATE: u32 = 44_100;
const AMPLITUDE: f32 = 0.08;

pub struct Audio {
    // Field order matters for drop order, and the stream must outlive the sink.
    sink: Sink,
    _handle: OutputStreamHandle,
    _stream: OutputStream,
}

impl Audio {
    /// Open the default output device. Returns `None` if no device is available
    /// (headless box, no ALSA, etc.) — the game then plays silently.
    pub fn new() -> Option<Self> {
        let (stream, handle) = OutputStream::try_default().ok()?;
        let sink = Sink::try_new(&handle).ok()?;
        Some(Audio { sink, _handle: handle, _stream: stream })
    }

    /// Queue a square-wave tone of `freq` Hz for `secs` seconds. A short
    /// attack/release envelope avoids speaker clicks. Skipped when muted, when
    /// the parameters are degenerate, or when the queue is already backed up.
    pub fn beep(&self, freq: f64, secs: f64, muted: bool) {
        if muted || freq <= 0.0 || secs <= 0.0 {
            return;
        }
        if self.sink.len() > 48 {
            return; // don't let a backlog of blips pile up
        }
        let secs = (secs as f32).clamp(0.004, 2.0);
        let total = (secs * SAMPLE_RATE as f32) as usize;
        if total == 0 {
            return;
        }
        let period = SAMPLE_RATE as f32 / freq as f32;
        let edge = (total / 12).max(1); // attack/release length in samples
        let mut data = Vec::with_capacity(total);
        for i in 0..total {
            let phase = (i as f32 % period) / period;
            let level = if phase < 0.5 { AMPLITUDE } else { -AMPLITUDE };
            let env = if i < edge {
                i as f32 / edge as f32
            } else if i + edge > total {
                (total - i) as f32 / edge as f32
            } else {
                1.0
            };
            data.push(level * env);
        }
        self.sink.append(SamplesBuffer::new(1, SAMPLE_RATE, data));
    }
}
