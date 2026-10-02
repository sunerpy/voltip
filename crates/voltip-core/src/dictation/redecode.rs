//! Live preview from a recogniser that only takes whole recordings (docs/dictation.md §11.8,
//! 2026-10-02). The built-in service's Qwen3-ASR answers whole files and has no streaming endpoint,
//! so the sentence being spoken is sent to it again as it grows: every [`RedecodeParams::step`] of
//! new audio, one request at a time. Its latest answer is the preview (`Partial`). A pause, or a
//! sentence [`RedecodeParams::longest`] long, closes the sentence: it is decoded once more, whole,
//! and that answer is final for it (`Endpoint`). This is the way Qwen3-ASR's own streaming works
//! (the audio so far goes in again), driven from the client against the endpoint every take
//! already uses. Measured 2026-10-02 against the built-in service: 1.2–2.5 s per answer for 2–8 s
//! of audio.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::time::{Duration, Instant};

use super::ports::{DictationError, LIVE_SAMPLE_RATE_HZ, Segment, StreamEvent, StreamFinal, StreamingSession, StreamingTranscriber, Transcriber};
use super::wav;

/// Samples in the 20 ms frame the pause detector looks at.
const FRAME: usize = LIVE_SAMPLE_RATE_HZ as usize / 50;
/// Silence after which no preview goes out: the sentence may be ending.
const PAUSING: Duration = Duration::from_millis(200);

/// When the preview asks again, and when a sentence ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RedecodeParams {
    /// New audio in the sentence before it is decoded again.
    pub step: Duration,
    /// Silence after speech that closes the sentence.
    pub pause: Duration,
    /// A sentence this long is closed even without a pause, so no request grows without end.
    pub longest: Duration,
    /// Silence kept before a sentence's first sound.
    pub pre_roll: Duration,
    /// RMS of a 20 ms frame (full scale = 1) below which the frame is silence.
    pub silence_rms: f32,
    /// How long [`StreamingSession::finish`] waits for the sentences still being decoded.
    pub finish_timeout: Duration,
}

impl Default for RedecodeParams {
    fn default() -> Self {
        Self {
            step: Duration::from_secs(1),
            pause: Duration::from_millis(800),
            longest: Duration::from_secs(20),
            pre_roll: Duration::from_millis(300),
            silence_rms: 0.01,
            finish_timeout: Duration::from_secs(30),
        }
    }
}

fn samples(d: Duration) -> usize {
    usize::try_from(d.as_millis() * u128::from(LIVE_SAMPLE_RATE_HZ) / 1000).unwrap_or(usize::MAX)
}

fn ms(samples: usize) -> u64 {
    u64::try_from(samples).unwrap_or(u64::MAX) * 1000 / u64::from(LIVE_SAMPLE_RATE_HZ)
}

/// 16 kHz `f32` as the 16-bit WAV a recogniser takes.
fn wav_of(pcm: &[f32]) -> Vec<u8> {
    let pcm16: Vec<i16> = pcm.iter().map(|&x| (x.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16).collect();
    wav::encode_pcm16(&pcm16, LIVE_SAMPLE_RATE_HZ)
}

/// The streaming port over a whole-take [`Transcriber`]: one per take, with the take's glossary.
pub struct RedecodeStreaming {
    transcriber: Arc<dyn Transcriber>,
    glossary: Arc<Vec<String>>,
    params: RedecodeParams,
    flush: bool,
}

impl std::fmt::Debug for RedecodeStreaming {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedecodeStreaming")
            .field("params", &self.params)
            .field("glossary", &self.glossary.len())
            .field("flush", &self.flush)
            .finish_non_exhaustive()
    }
}

impl RedecodeStreaming {
    /// Preview with `transcriber`, hinted with `glossary` as the take is.
    pub fn new(transcriber: Arc<dyn Transcriber>, glossary: Vec<String>) -> Self {
        Self::with_params(transcriber, glossary, RedecodeParams::default())
    }

    /// [`RedecodeStreaming::new`] with other timings (tests).
    pub fn with_params(transcriber: Arc<dyn Transcriber>, glossary: Vec<String>, params: RedecodeParams) -> Self {
        Self { transcriber, glossary: Arc::new(glossary), params, flush: true }
    }

    /// Whether [`StreamingSession::finish`] decodes what is left. The streaming output modes take
    /// their text from it; a whole take does not, since the take's transcriber decodes the same
    /// audio anyway: there the last preview is the tail and nothing more is sent.
    pub fn flushing(mut self, flush: bool) -> Self {
        self.flush = flush;
        self
    }

    fn session(&self, rt: tokio::runtime::Handle, language: Option<&str>) -> RedecodeSession {
        RedecodeSession {
            rt,
            transcriber: self.transcriber.clone(),
            glossary: self.glossary.clone(),
            language: language.map(str::to_owned),
            params: self.params,
            flush: self.flush,
            fed: 0,
            frame_energy: 0.0,
            frame_len: 0,
            current: Sentence::new(0, 0),
            next_id: 1,
            closing: VecDeque::new(),
            running: None,
            events: VecDeque::new(),
            committed: Vec::new(),
            tail: String::new(),
            last_preview: String::new(),
            failure: None,
        }
    }
}

impl StreamingTranscriber for RedecodeStreaming {
    fn open(&self, language: Option<&str>) -> Result<Box<dyn StreamingSession>, DictationError> {
        // The decode thread is a blocking task of the core's runtime: the requests go there.
        let rt = tokio::runtime::Handle::try_current().map_err(|_| DictationError::Asr("live preview: no runtime for the requests".into()))?;
        Ok(Box::new(self.session(rt, language)))
    }

    /// Nothing to load: the recogniser is a remote service.
    fn warm(&self) {}
}

/// The sentence being spoken.
struct Sentence {
    id: u64,
    /// Stream sample of `samples[0]`.
    start: usize,
    samples: Vec<f32>,
    /// A frame above the silence level has been heard.
    voiced: bool,
    /// Samples of silence at the end.
    silence: usize,
    /// `samples.len()` when it last went out for a preview.
    sent: usize,
}

impl Sentence {
    fn new(id: u64, start: usize) -> Self {
        Self { id, start, samples: Vec::new(), voiced: false, silence: 0, sent: 0 }
    }
}

enum Job {
    /// A preview of sentence `sentence`; its answer is dropped once another sentence is current.
    Preview { sentence: u64 },
    /// A closed sentence's final decode; `last` is the one open when the take ended (the tail).
    Close { start_ms: u64, end_ms: u64, last: bool },
}

struct Running {
    job: Job,
    answer: mpsc::Receiver<Result<String, DictationError>>,
}

struct RedecodeSession {
    rt: tokio::runtime::Handle,
    transcriber: Arc<dyn Transcriber>,
    glossary: Arc<Vec<String>>,
    language: Option<String>,
    params: RedecodeParams,
    flush: bool,
    /// Stream samples so far.
    fed: usize,
    frame_energy: f32,
    frame_len: usize,
    current: Sentence,
    next_id: u64,
    /// Closed sentences waiting for their final decode, in order.
    closing: VecDeque<(Job, Vec<u8>)>,
    /// The one request out.
    running: Option<Running>,
    events: VecDeque<StreamEvent>,
    committed: Vec<Segment>,
    tail: String,
    last_preview: String,
    failure: Option<DictationError>,
}

impl RedecodeSession {
    fn on_frame(&mut self, rms: f32) {
        if rms >= self.params.silence_rms {
            self.current.voiced = true;
            self.current.silence = 0;
        } else {
            self.current.silence += FRAME;
        }
        if !self.current.voiced {
            // Silence before any speech: only the pre-roll is kept.
            let keep = samples(self.params.pre_roll);
            if self.current.samples.len() > keep {
                let drop = self.current.samples.len() - keep;
                self.current.samples.drain(..drop);
                self.current.start += drop;
            }
            return;
        }
        if self.current.silence >= samples(self.params.pause) || self.current.samples.len() >= samples(self.params.longest) {
            self.close_current(false);
        }
    }

    /// End the current sentence here; a voiced one waits for its final decode.
    fn close_current(&mut self, last: bool) {
        let next = Sentence::new(self.next_id, self.fed);
        self.next_id += 1;
        let done = std::mem::replace(&mut self.current, next);
        if !done.voiced {
            return;
        }
        let spoken = done.samples.len() - done.silence.min(done.samples.len());
        let job = Job::Close { start_ms: ms(done.start), end_ms: ms(done.start + spoken), last };
        self.closing.push_back((job, wav_of(&done.samples)));
    }

    /// Send the next request when none is out: a closed sentence first, else a preview of the
    /// current one once it has grown by a step.
    fn pump(&mut self) {
        if self.running.is_some() || self.failure.is_some() {
            return;
        }
        if let Some((job, wav)) = self.closing.pop_front() {
            self.send(job, wav);
            return;
        }
        let s = &mut self.current;
        // In a pause already (it may be the end of the sentence): the close decodes it soon.
        let pausing = s.silence >= samples(PAUSING);
        if s.voiced && !pausing && s.samples.len() >= s.sent + samples(self.params.step) {
            s.sent = s.samples.len();
            let (job, wav) = (Job::Preview { sentence: s.id }, wav_of(&s.samples));
            self.send(job, wav);
        }
    }

    fn send(&mut self, job: Job, wav: Vec<u8>) {
        let (tx, answer) = mpsc::channel();
        let (transcriber, glossary, language) = (self.transcriber.clone(), self.glossary.clone(), self.language.clone());
        self.rt.spawn(async move {
            let result = transcriber.transcribe(&wav, language.as_deref(), &glossary).await.map(|t| t.text);
            let _ = tx.send(result);
        });
        self.running = Some(Running { job, answer });
    }

    fn settle(&mut self, job: Job, answer: Result<String, DictationError>) {
        match (job, answer) {
            (Job::Preview { sentence }, Ok(text)) => {
                let text = text.trim();
                if sentence == self.current.id && !text.is_empty() && text != self.last_preview {
                    self.last_preview = text.to_owned();
                    self.events.push_back(StreamEvent::Partial { current: text.to_owned() });
                }
            }
            // A preview is only a preview: the next step asks again.
            (Job::Preview { .. }, Err(e)) => tracing::debug!(error = %e, "live preview request failed"),
            (Job::Close { last: true, .. }, Ok(text)) => self.tail = text.trim().to_owned(),
            (Job::Close { start_ms, end_ms, last: false }, Ok(text)) => {
                self.last_preview.clear();
                let text = text.trim().to_owned();
                if !text.is_empty() {
                    self.committed.push(Segment { text: text.clone(), start_ms, end_ms });
                    self.events.push_back(StreamEvent::Endpoint { text, start_ms, end_ms });
                }
            }
            (Job::Close { .. }, Err(e)) => self.fail(e),
        }
    }

    fn fail(&mut self, error: DictationError) {
        if self.failure.is_none() {
            self.events.push_back(StreamEvent::Error(error.to_string()));
            self.failure = Some(error);
        }
    }
}

impl StreamingSession for RedecodeSession {
    fn feed(&mut self, pcm16k: &[f32]) {
        for &x in pcm16k {
            self.current.samples.push(x);
            self.fed += 1;
            self.frame_energy += x * x;
            self.frame_len += 1;
            if self.frame_len == FRAME {
                let rms = (self.frame_energy / FRAME as f32).sqrt();
                (self.frame_energy, self.frame_len) = (0.0, 0);
                self.on_frame(rms);
            }
        }
        self.pump();
    }

    fn poll(&mut self) -> StreamEvent {
        if let Some(running) = &self.running {
            match running.answer.try_recv() {
                Ok(answer) => {
                    if let Some(Running { job, .. }) = self.running.take() {
                        self.settle(job, answer);
                    }
                    self.pump();
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.running = None;
                    self.fail(DictationError::Asr("live preview: the request was dropped".into()));
                }
            }
        }
        self.events.pop_front().unwrap_or(StreamEvent::Idle)
    }

    fn finish(mut self: Box<Self>) -> Result<StreamFinal, DictationError> {
        if !self.flush {
            // A whole take: its transcriber has the audio; the preview so far is all there is.
            return Ok(StreamFinal { committed: std::mem::take(&mut self.committed), tail: std::mem::take(&mut self.last_preview) });
        }
        self.close_current(true);
        let deadline = Instant::now() + self.params.finish_timeout;
        loop {
            if let Some(error) = &self.failure {
                return Err(error.clone());
            }
            if self.running.is_none() {
                match self.closing.pop_front() {
                    Some((job, wav)) => self.send(job, wav),
                    None => break,
                }
            }
            let Some(running) = &self.running else { break };
            match running.answer.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(answer) => {
                    if let Some(Running { job, .. }) = self.running.take() {
                        self.settle(job, answer);
                    }
                }
                Err(RecvTimeoutError::Timeout) => return Err(DictationError::Asr(format!("flush: no answer within {:?}", self.params.finish_timeout))),
                Err(RecvTimeoutError::Disconnected) => return Err(DictationError::Asr("flush: the request was dropped".into())),
            }
        }
        Ok(StreamFinal { committed: std::mem::take(&mut self.committed), tail: std::mem::take(&mut self.tail) })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::dictation::fakes::FakeTranscriber;

    /// `secs` of a 440 Hz tone at a third of full scale (speech to the pause detector).
    fn tone(secs: f32) -> Vec<f32> {
        (0..(secs * LIVE_SAMPLE_RATE_HZ as f32) as usize).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / LIVE_SAMPLE_RATE_HZ as f32).sin() / 3.0).collect()
    }

    fn quiet(secs: f32) -> Vec<f32> {
        vec![0.0; (secs * LIVE_SAMPLE_RATE_HZ as f32) as usize]
    }

    fn session(transcriber: &Arc<FakeTranscriber>, params: RedecodeParams) -> RedecodeSession {
        RedecodeStreaming::with_params(transcriber.clone(), vec!["Voltip".into()], params).session(tokio::runtime::Handle::current(), Some("zh"))
    }

    fn busy(s: &RedecodeSession) -> bool {
        s.running.is_some() || !s.closing.is_empty()
    }

    /// Poll until nothing is out any more, collecting the events.
    fn settle(s: &mut RedecodeSession, events: &mut Vec<StreamEvent>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match s.poll() {
                StreamEvent::Idle if !busy(s) => return,
                // An answer on its way: a short back-off, not a wait for anything in particular.
                StreamEvent::Idle => std::thread::sleep(Duration::from_millis(1)),
                event => events.push(event),
            }
            assert!(Instant::now() < deadline, "no answer within 10 s");
        }
    }

    /// Feed `pcm` in the decode thread's 100 ms chunks, letting every request a chunk starts come
    /// back before the next chunk, so what each one asks for is deterministic.
    fn drive(s: &mut RedecodeSession, pcm: &[f32], events: &mut Vec<StreamEvent>) {
        for chunk in pcm.chunks(1600) {
            s.feed(chunk);
            settle(s, events);
        }
    }

    /// Run `body` on a blocking thread of a runtime, as the core's decode thread is.
    async fn on_decode_thread<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> T {
        tokio::task::spawn_blocking(body).await.unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_sentence_is_decoded_again_as_it_grows_and_a_pause_closes_it() {
        let transcriber = Arc::new(FakeTranscriber::numbered(0, &[], Duration::ZERO));
        let t = transcriber.clone();
        let (events, fin) = on_decode_thread(move || {
            let mut s = session(&t, RedecodeParams::default());
            let mut events = Vec::new();
            // 0.5 s of silence (only the pre-roll is kept), 2.5 s of speech, 1 s of silence, 1.4 s more.
            drive(&mut s, &quiet(0.5), &mut events);
            drive(&mut s, &tone(2.5), &mut events);
            drive(&mut s, &quiet(1.0), &mut events);
            drive(&mut s, &tone(1.4), &mut events);
            (events, Box::new(s).finish().unwrap())
        })
        .await;
        // Every second of new audio went out once, one request at a time, and no preview during
        // the pause; the pause closed the sentence with one more, whole decode.
        assert_eq!(
            events,
            [
                StreamEvent::Partial { current: "第1段。".into() },
                StreamEvent::Partial { current: "第2段。".into() },
                StreamEvent::Endpoint { text: "第3段。".into(), start_ms: 200, end_ms: 3000 },
                StreamEvent::Partial { current: "第4段。".into() },
            ]
        );
        // The open sentence at the end is the tail; the closed one stays committed.
        assert_eq!(fin, StreamFinal { committed: vec![Segment { text: "第3段。".into(), start_ms: 200, end_ms: 3000 }], tail: "第5段。".into() });
        // The previews grew by a second from the pre-roll on; the close carried the sentence with
        // its pause; the second sentence starts with the 0.2 s of silence before it.
        assert_eq!(transcriber.durations_ms(), [1000, 2000, 3600, 1000, 1600]);
        assert!(transcriber.languages().iter().all(|l| l.as_deref() == Some("zh")));
        assert!(transcriber.glossaries().iter().all(|g| g == &["Voltip".to_owned()]));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn one_request_at_a_time_and_a_preview_of_a_closed_sentence_is_dropped() {
        let transcriber = Arc::new(FakeTranscriber::slow("你好。", Duration::from_millis(300)));
        let t = transcriber.clone();
        let (events, fin) = on_decode_thread(move || {
            let mut s = session(&t, RedecodeParams::default());
            let mut events = Vec::new();
            // Fed far faster than the recogniser answers: the first preview is the only request
            // out until it is back, and the pause closes the sentence meanwhile.
            for chunk in [tone(1.5), quiet(1.0)].concat().chunks(1600) {
                s.feed(chunk);
                if let event @ (StreamEvent::Partial { .. } | StreamEvent::Endpoint { .. } | StreamEvent::Error(_)) = s.poll() {
                    events.push(event);
                }
            }
            assert_eq!(t.calls(), 1, "nothing overtakes the request out");
            settle(&mut s, &mut events);
            (events, Box::new(s).finish().unwrap())
        })
        .await;
        // The preview answered after its sentence closed: dropped; the close's answer is final.
        assert_eq!(events, [StreamEvent::Endpoint { text: "你好。".into(), start_ms: 0, end_ms: 1500 }]);
        assert_eq!(fin.committed.len(), 1);
        assert!(fin.tail.is_empty(), "nothing was said after the pause");
        assert_eq!(transcriber.durations_ms(), [1000, 2300]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_preview_is_asked_again_and_a_failed_close_ends_the_session() {
        // Call 1 (the first preview) fails: nothing is shown, the next step asks again.
        let transcriber = Arc::new(FakeTranscriber::numbered(0, &[1], Duration::ZERO));
        let t = transcriber.clone();
        let events = on_decode_thread(move || {
            let mut s = session(&t, RedecodeParams::default());
            let mut events = Vec::new();
            drive(&mut s, &tone(2.0), &mut events);
            events
        })
        .await;
        assert_eq!(events, [StreamEvent::Partial { current: "第2段。".into() }]);
        // Call 2 (the close) fails: the session reports it once and the flush fails too.
        let transcriber = Arc::new(FakeTranscriber::numbered(0, &[2], Duration::ZERO));
        let t = transcriber.clone();
        let (events, fin) = on_decode_thread(move || {
            let mut s = session(&t, RedecodeParams::default());
            let mut events = Vec::new();
            drive(&mut s, &[tone(1.0), quiet(1.0), tone(1.0)].concat(), &mut events);
            (events, Box::new(s).finish())
        })
        .await;
        assert_eq!(events, [StreamEvent::Partial { current: "第1段。".into() }, StreamEvent::Error("asr: fake failure of call 2".into())]);
        assert_eq!(fin, Err(DictationError::Asr("fake failure of call 2".into())));
        assert_eq!(transcriber.calls(), 2, "nothing more goes out after the failure");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_sentence_without_a_pause_is_closed_at_its_longest_and_the_flush_waits_its_turn() {
        let transcriber = Arc::new(FakeTranscriber::numbered(0, &[], Duration::ZERO));
        let params = RedecodeParams { step: Duration::from_secs(10), longest: Duration::from_secs(2), ..RedecodeParams::default() };
        let t = transcriber.clone();
        let (events, fin) = on_decode_thread(move || {
            let mut s = session(&t, params);
            let mut events = Vec::new();
            drive(&mut s, &tone(5.0), &mut events);
            (events, Box::new(s).finish().unwrap())
        })
        .await;
        assert_eq!(
            events,
            [
                StreamEvent::Endpoint { text: "第1段。".into(), start_ms: 0, end_ms: 2000 },
                StreamEvent::Endpoint { text: "第2段。".into(), start_ms: 2000, end_ms: 4000 },
            ]
        );
        assert_eq!(fin.tail, "第3段。");
        assert_eq!(transcriber.durations_ms(), [2000, 2000, 1000]);
        // A flush whose answer does not come in time fails, which degrades the preview.
        let slow = Arc::new(FakeTranscriber::slow("迟到。", Duration::from_secs(5)));
        let params = RedecodeParams { finish_timeout: Duration::from_millis(100), ..RedecodeParams::default() };
        let late = on_decode_thread(move || {
            let mut s = session(&slow, params);
            s.feed(&tone(0.5));
            Box::new(s).finish()
        })
        .await;
        assert_eq!(late, Err(DictationError::Asr("flush: no answer within 100ms".into())));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn without_the_flush_the_last_preview_is_the_tail_and_nothing_more_goes_out() {
        let transcriber = Arc::new(FakeTranscriber::numbered(0, &[], Duration::ZERO));
        let t = transcriber.clone();
        let fin = on_decode_thread(move || {
            let mut s = RedecodeStreaming::new(t.clone(), Vec::new()).flushing(false).session(tokio::runtime::Handle::current(), None);
            drive(&mut s, &tone(1.5), &mut Vec::new());
            Box::new(s).finish().unwrap()
        })
        .await;
        assert_eq!(fin, StreamFinal { committed: Vec::new(), tail: "第1段。".into() });
        assert_eq!(transcriber.durations_ms(), [1000], "the preview only");
    }

    #[test]
    fn opening_needs_the_runtime_the_requests_go_to() {
        let streaming = RedecodeStreaming::new(Arc::new(FakeTranscriber::ok("x")), Vec::new());
        assert!(matches!(streaming.open(None), Err(DictationError::Asr(m)) if m.contains("no runtime")));
        streaming.warm();
        assert!(format!("{streaming:?}").contains("RedecodeStreaming"));
        assert_eq!((samples(Duration::from_millis(800)), ms(16_000)), (12_800, 1000));
    }
}
