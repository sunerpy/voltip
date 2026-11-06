//! The audio a streaming session sent its service, for the reason a take gives when the session
//! ended without any text (docs/dictation.md §3.15): how long it was and how loud it got, so the
//! take's history tells a silent microphone from a service that answered nothing.

/// The sample rate of what a session is fed.
const SAMPLE_RATE_HZ: f64 = 16_000.0;
/// Below this peak (60 dB under full scale) the audio is as good as silent.
const NEARLY_SILENT: f32 = 0.001;

/// What was fed to one session.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Fed {
    samples: usize,
    /// The loudest sample, as a fraction of full scale.
    peak: f32,
}

impl Fed {
    /// Count `pcm16k`, 16 kHz samples in `-1.0..=1.0`.
    pub(crate) fn add(&mut self, pcm16k: &[f32]) {
        self.samples += pcm16k.len();
        self.peak = pcm16k.iter().fold(self.peak, |peak, x| peak.max(x.abs().min(1.0)));
    }

    /// The note of a session that ended without text: the service's own remark, then the audio.
    pub(crate) fn note(&self, remark: Option<&str>) -> String {
        match remark.map(str::trim).filter(|remark| !remark.is_empty()) {
            Some(remark) => format!("{remark}；{}", self.describe()),
            None => self.describe(),
        }
    }

    /// The audio in the words of the take's history.
    fn describe(&self) -> String {
        if self.samples == 0 {
            return "未发送音频".to_owned();
        }
        let seconds = self.samples as f64 / SAMPLE_RATE_HZ;
        if self.peak <= 0.0 {
            return format!("已发送 {seconds:.1} 秒音频，均为静音");
        }
        let db = (20.0 * f64::from(self.peak).log10()).round() as i32;
        let quiet = if self.peak < NEARLY_SILENT { "，几乎没有声音" } else { "" };
        format!("已发送 {seconds:.1} 秒音频，最大音量 {db} dB{quiet}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fed(chunks: &[&[f32]]) -> Fed {
        let mut fed = Fed::default();
        for chunk in chunks {
            fed.add(chunk);
        }
        fed
    }

    /// The audio's length and its loudest sample over every chunk; silence and no audio at all
    /// say so, and the service's remark comes first.
    #[test]
    fn the_note_says_how_long_and_how_loud_the_audio_was() {
        assert_eq!(Fed::default().note(None), "未发送音频");
        assert_eq!(fed(&[&[0.0; 16_000], &[0.0; 8_000]]).note(None), "已发送 1.5 秒音频，均为静音");
        assert_eq!(fed(&[&[0.0; 8_000], &[0.25, -0.5], &[0.1; 7_998]]).note(None), "已发送 1.0 秒音频，最大音量 -6 dB");
        // Clipped samples count as full scale.
        assert_eq!(fed(&[&[1.5; 16_000]]).note(None), "已发送 1.0 秒音频，最大音量 0 dB");
        assert_eq!(fed(&[&[0.0005; 32_000]]).note(None), "已发送 2.0 秒音频，最大音量 -66 dB，几乎没有声音");
        let loud = fed(&[&[-0.5; 16_000]]);
        assert_eq!(loud.note(Some(" 豆包语音返回了 3 次结果，均不含文字 ")), "豆包语音返回了 3 次结果，均不含文字；已发送 1.0 秒音频，最大音量 -6 dB");
        assert_eq!(loud.note(Some("  ")), "已发送 1.0 秒音频，最大音量 -6 dB");
    }
}
