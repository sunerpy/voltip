//! The recognition services with protocols of their own (docs/dictation.md §3.15) as the core's
//! ports: a [`voltip_asr::vendor::VendorAsr`] behind [`Transcriber`], and for the services that
//! recognise while the take is spoken behind [`StreamingTranscriber`] too.
//!
//! As with Model Studio's realtime models ([`crate::dashscope`]), a streaming session runs in a task
//! on the core's runtime: `feed` queues 16-bit PCM for it, `poll` reads what the service said,
//! `finish` asks for the last sentence and waits for the session to end. A session dropped without
//! `finish` (a cancelled take) closes its input; the client then closes the connection.

use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;
use voltip_asr::AsrError;
use voltip_asr::vendor::{RealtimeEvent, RealtimeInput, Vendor, VendorAsr};
use voltip_core::AsrProtocol;
use voltip_core::dictation::{DictationError, Segment, StreamEvent, StreamFinal, StreamingSession, StreamingTranscriber, Transcriber, Transcript};

use crate::dashscope::FINISH_TIMEOUT;
use crate::fed::Fed;

/// The vendor client of `protocol`, when it is one of theirs.
pub fn vendor_of(protocol: AsrProtocol) -> Option<Vendor> {
    Some(match protocol {
        AsrProtocol::VolcengineSauc => Vendor::Volcengine,
        AsrProtocol::TencentRealtime => Vendor::Tencent,
        AsrProtocol::IflytekIat => Vendor::IflytekIat,
        AsrProtocol::IflytekRtasr => Vendor::IflytekRtasr,
        AsrProtocol::BaiduRealtime => Vendor::Baidu,
        AsrProtocol::StepfunTranscriptions => Vendor::StepfunTranscriptions,
        AsrProtocol::StepfunRealtime => Vendor::StepfunRealtime,
        AsrProtocol::ZhipuTranscriptions => Vendor::Zhipu,
        AsrProtocol::MimoChat => Vendor::Mimo,
        AsrProtocol::OpenaiTranscriptions
        | AsrProtocol::DashscopeChat
        | AsrProtocol::DashscopeMultimodal
        | AsrProtocol::DashscopeDuplex
        | AsrProtocol::DashscopeUnsupported => return None,
    })
}

/// A vendor's model as the take's recogniser; a realtime one streams as well.
pub struct VendorTranscriber {
    client: Arc<dyn VendorAsr>,
}

impl VendorTranscriber {
    /// The client of `vendor` for `config`; fails on a configuration it cannot use.
    pub fn new(vendor: Vendor, config: voltip_asr::AsrConfig) -> Result<Self, DictationError> {
        Ok(Self { client: voltip_asr::vendor::client(vendor, config).map_err(crate::asr_error)? })
    }

    /// The client itself (the probe's).
    pub fn client(&self) -> &Arc<dyn VendorAsr> {
        &self.client
    }
}

#[async_trait]
impl Transcriber for VendorTranscriber {
    /// A whole take, the glossary as the service takes it (docs/dictation.md §16.3).
    async fn transcribe(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, DictationError> {
        let t = self.client.transcribe(wav, language, glossary).await.map_err(crate::asr_error)?;
        Ok(Transcript {
            text: t.text,
            latency_ms: t.latency_ms,
            model: Some(t.model),
            backend_note: None,
            provider: None,
            fallback: None,
            glossary_sent: self.client.sends_glossary(glossary),
        })
    }

    fn streaming(&self, glossary: &[String]) -> Option<Arc<dyn StreamingTranscriber>> {
        self.client.streams().then(|| {
            Arc::new(VendorStreaming { client: Arc::clone(&self.client), glossary: glossary.to_vec(), finish_timeout: FINISH_TIMEOUT })
                as Arc<dyn StreamingTranscriber>
        })
    }
}

/// A vendor's realtime sessions for one take, with the take's glossary.
pub struct VendorStreaming {
    client: Arc<dyn VendorAsr>,
    glossary: Vec<String>,
    finish_timeout: Duration,
}

impl StreamingTranscriber for VendorStreaming {
    /// Starts the session in the background and returns at once: the audio fed meanwhile waits for
    /// the connection, and one that fails reports through `poll`.
    fn open(&self, language: Option<&str>) -> Result<Box<dyn StreamingSession>, DictationError> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| DictationError::Asr("realtime model: no runtime for the connection".into()))?;
        let (client, glossary, language) = (Arc::clone(&self.client), self.glossary.clone(), language.map(str::to_owned));
        let model = client.model().to_owned();
        let glossary_sent = client.sends_glossary(&glossary);
        let (input_tx, input) = mpsc::unbounded_channel();
        let (events_tx, events) = std_mpsc::channel();
        let (done_tx, done) = std_mpsc::sync_channel(1);
        runtime.spawn(async move {
            let result = drive(client.as_ref(), language.as_deref(), &glossary, input, &events_tx).await;
            if let Err(e) = &result {
                let _ = events_tx.send(StreamEvent::Error(e.to_string()));
            }
            let _ = done_tx.send(result.map_err(crate::asr_error));
        });
        Ok(Box::new(VendorSession { input: input_tx, events, done, finish_timeout: self.finish_timeout, model, glossary_sent, fed: Fed::default() }))
    }

    /// Nothing to load: the model is the service's.
    fn warm(&self) {}
}

struct VendorSession {
    input: mpsc::UnboundedSender<RealtimeInput>,
    events: std_mpsc::Receiver<StreamEvent>,
    done: std_mpsc::Receiver<Result<StreamFinal, DictationError>>,
    finish_timeout: Duration,
    /// The model the session streams to: its text's model (docs/dictation.md §3.5).
    model: String,
    /// Its requests carry the glossary (docs/dictation.md §30.6).
    glossary_sent: bool,
    /// The audio it was fed.
    fed: Fed,
}

impl StreamingSession for VendorSession {
    fn feed(&mut self, pcm16k: &[f32]) {
        self.fed.add(pcm16k);
        let pcm: Vec<u8> = pcm16k.iter().flat_map(|&x| ((x.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16).to_le_bytes()).collect();
        // A session that already ended has said why through `poll`.
        let _ = self.input.send(RealtimeInput::Audio(pcm));
    }

    fn poll(&mut self) -> StreamEvent {
        self.events.try_recv().unwrap_or(StreamEvent::Idle)
    }

    fn model(&self) -> Option<String> {
        Some(self.model.clone())
    }

    fn glossary_sent(&self) -> Option<bool> {
        Some(self.glossary_sent)
    }

    /// The session's last sentences; one that gave no text at all says what the service answered
    /// and the audio it was sent.
    fn finish(self: Box<Self>) -> Result<StreamFinal, DictationError> {
        let _ = self.input.send(RealtimeInput::Finish);
        match self.done.recv_timeout(self.finish_timeout) {
            Ok(result) => result.map(|fin| {
                let silent = fin.committed.is_empty() && fin.tail.trim().is_empty();
                let note = silent.then(|| self.fed.note(fin.note.as_deref()));
                StreamFinal { glossary_sent: Some(self.glossary_sent), note, ..fin }
            }),
            // docs/dictation.md §3.14: the service went silent, a timeout.
            Err(_) => Err(DictationError::Unreachable {
                service: voltip_core::ServiceKind::Asr,
                cause: voltip_core::dictation::Outage::Timeout,
                detail: format!("realtime model: no last sentence within {} s", self.finish_timeout.as_secs()),
            }),
        }
    }
}

/// One session: the client's, with what it says passed on as the preview's events and folded into
/// the take's text. A session that ended without saying it finished is an error.
async fn drive(
    client: &dyn VendorAsr,
    language: Option<&str>,
    glossary: &[String],
    input: mpsc::UnboundedReceiver<RealtimeInput>,
    events: &std_mpsc::Sender<StreamEvent>,
) -> Result<StreamFinal, AsrError> {
    let (out_tx, mut out) = mpsc::unbounded_channel();
    let session = client.realtime(language, glossary, input, out_tx);
    let fold = async {
        let (mut committed, mut current, mut finished, mut note) = (Vec::new(), String::new(), false, None);
        while let Some(event) = out.recv().await {
            match event {
                RealtimeEvent::Partial { text } => {
                    current.clone_from(&text);
                    let _ = events.send(StreamEvent::Partial { current: text });
                }
                RealtimeEvent::Sentence { text, begin_ms, end_ms } => {
                    current.clear();
                    committed.push(Segment { text: text.clone(), refined: None, start_ms: begin_ms, end_ms });
                    let _ = events.send(StreamEvent::Endpoint { text, start_ms: begin_ms, end_ms });
                }
                RealtimeEvent::Note { text } => note = Some(text),
                RealtimeEvent::Finished => {
                    finished = true;
                    break;
                }
            }
        }
        (committed, current, finished, note)
    };
    let (result, (committed, current, finished, note)) = tokio::join!(session, fold);
    result?;
    if !finished {
        return Err(voltip_asr::realtime::closed_early());
    }
    // A sentence the service never closed is the tail.
    Ok(StreamFinal { committed, tail: current, model: Some(client.model().to_owned()), provider: None, fallback: None, glossary_sent: None, note })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// A scripted realtime client: a partial per packet of audio and a sentence at `Finish`; or
    /// failing, or ending without `Finished`, or without any text.
    struct Scripted {
        fail: Option<AsrError>,
        finishes: bool,
        audio: Arc<Mutex<usize>>,
        says: Says,
    }

    /// What a scripted session says.
    #[derive(Clone, Copy)]
    enum Says {
        Text,
        /// No text at all; its remark at the end, when it has one.
        Nothing(Option<&'static str>),
    }

    #[async_trait]
    impl VendorAsr for Scripted {
        fn model(&self) -> &str {
            "scripted"
        }

        fn sends_glossary(&self, glossary: &[String]) -> bool {
            !glossary.is_empty()
        }

        async fn transcribe(&self, _wav: &[u8], _language: Option<&str>, _glossary: &[String]) -> Result<voltip_asr::Transcript, AsrError> {
            match &self.fail {
                Some(e) => Err(e.clone()),
                None => Ok(voltip_asr::Transcript { text: "整段".into(), latency_ms: 1, model: "scripted".into() }),
            }
        }

        async fn probe(&self) -> Result<(), AsrError> {
            self.fail.clone().map_or(Ok(()), Err)
        }

        fn streams(&self) -> bool {
            true
        }

        async fn realtime(
            &self,
            _language: Option<&str>,
            _glossary: &[String],
            mut input: mpsc::UnboundedReceiver<RealtimeInput>,
            events: voltip_asr::vendor::RealtimeEvents,
        ) -> Result<(), AsrError> {
            if let Some(e) = &self.fail {
                return Err(e.clone());
            }
            let mut sent = 0usize;
            while let Some(item) = input.recv().await {
                match item {
                    RealtimeInput::Audio(pcm) => {
                        sent += pcm.len();
                        *self.audio.lock().unwrap() += pcm.len();
                        if matches!(self.says, Says::Text) {
                            let _ = events.send(RealtimeEvent::Partial { text: "你".into() });
                        }
                    }
                    RealtimeInput::Finish => {
                        let end = u64::try_from(sent / 32).unwrap();
                        match self.says {
                            Says::Text => {
                                let _ = events.send(RealtimeEvent::Sentence { text: "你好。".into(), begin_ms: 0, end_ms: end });
                            }
                            Says::Nothing(Some(remark)) => {
                                let _ = events.send(RealtimeEvent::Note { text: remark.into() });
                            }
                            Says::Nothing(None) => {}
                        }
                        if self.finishes {
                            let _ = events.send(RealtimeEvent::Finished);
                        }
                        return Ok(());
                    }
                }
            }
            Err(AsrError::Network("cancelled".into()))
        }
    }

    fn streaming(fail: Option<AsrError>, finishes: bool) -> (Arc<dyn StreamingTranscriber>, Arc<Mutex<usize>>) {
        scripted(fail, finishes, Says::Text)
    }

    fn scripted(fail: Option<AsrError>, finishes: bool, says: Says) -> (Arc<dyn StreamingTranscriber>, Arc<Mutex<usize>>) {
        let audio = Arc::new(Mutex::new(0));
        let client: Arc<dyn VendorAsr> = Arc::new(Scripted { fail, finishes, audio: Arc::clone(&audio), says });
        let t = VendorTranscriber { client };
        (t.streaming(&["术语".into()]).expect("a realtime client streams"), audio)
    }

    /// The session's sentences are the take's text, its model the receipt, and the glossary is
    /// said to have gone out.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_session_streams_its_sentences_and_finishes() {
        let (stream, audio) = streaming(None, true);
        let fin = tokio::task::spawn_blocking(move || {
            let mut session = stream.open(Some("zh")).unwrap();
            session.feed(&[0.1; 1600]);
            assert_eq!(session.model().as_deref(), Some("scripted"));
            assert_eq!(session.glossary_sent(), Some(true));
            session.finish()
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(*audio.lock().unwrap(), 3200, "100 ms of 16-bit PCM");
        assert_eq!(fin.committed.len(), 1);
        assert_eq!((fin.committed[0].text.as_str(), fin.committed[0].end_ms), ("你好。", 100));
        assert_eq!((fin.model.as_deref(), fin.glossary_sent), (Some("scripted"), Some(true)));
        assert_eq!(fin.note, None, "a session with text needs no note");
    }

    /// regression: a session that ended without any text left the take's history with "实时识别未得到
    /// 文本" alone, which tells neither the user nor support what happened. It now says what the
    /// service remarked and the audio it was sent (docs/dictation.md §12).
    #[tokio::test(flavor = "multi_thread")]
    async fn regression_a_session_without_text_says_what_it_was_sent() {
        for (says, want) in [
            (Says::Nothing(Some("豆包语音返回了 2 次结果，均不含文字")), "豆包语音返回了 2 次结果，均不含文字；已发送 0.2 秒音频，最大音量 -20 dB"),
            (Says::Nothing(None), "已发送 0.2 秒音频，最大音量 -20 dB"),
        ] {
            let (stream, _) = scripted(None, true, says);
            let fin = tokio::task::spawn_blocking(move || {
                let mut session = stream.open(None).unwrap();
                session.feed(&[0.1; 1600]);
                session.feed(&[-0.1; 1600]);
                session.finish()
            })
            .await
            .unwrap()
            .unwrap();
            assert!(fin.committed.is_empty() && fin.tail.is_empty(), "{fin:?}");
            assert_eq!(fin.note.as_deref(), Some(want));
        }
    }

    /// A failed session says why through `poll` and keeps the error's kind for `finish`; one that
    /// ends without saying it finished is an error too.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_session_reports_and_keeps_the_kind() {
        let quota = AsrError::QuotaExhausted { code: "x".into(), message: "used up".into(), retry_at_ms: None };
        let (stream, _) = streaming(Some(quota), true);
        let (polled, fin) = tokio::task::spawn_blocking(move || {
            let mut session = stream.open(None).unwrap();
            let polled = loop {
                match session.poll() {
                    StreamEvent::Idle => std::thread::sleep(Duration::from_millis(5)),
                    other => break other,
                }
            };
            (polled, session.finish())
        })
        .await
        .unwrap();
        assert!(matches!(polled, StreamEvent::Error(ref e) if e.contains("used up")), "{polled:?}");
        assert!(fin.as_ref().is_err_and(DictationError::is_quota_exhausted), "{fin:?}");

        let (stream, _) = streaming(None, false);
        let fin = tokio::task::spawn_blocking(move || stream.open(None).unwrap().finish()).await.unwrap();
        assert!(matches!(fin, Err(DictationError::Unreachable { .. })), "{fin:?}");
    }

    #[tokio::test]
    async fn a_whole_take_keeps_its_receipt_and_error_kinds() {
        let audio = Arc::new(Mutex::new(0));
        let t = VendorTranscriber { client: Arc::new(Scripted { fail: None, finishes: true, audio: Arc::clone(&audio), says: Says::Text }) };
        let ok = t.transcribe(b"wav", None, &["术语".into()]).await.unwrap();
        assert_eq!((ok.text.as_str(), ok.model.as_deref(), ok.glossary_sent), ("整段", Some("scripted"), true));
        for (error, check) in [
            (
                AsrError::QuotaExhausted { code: "c".into(), message: "m".into(), retry_at_ms: None },
                DictationError::is_quota_exhausted as fn(&DictationError) -> bool,
            ),
            (AsrError::RateLimited { retry_after_ms: None, message: "m".into() }, DictationError::is_outage),
            (AsrError::Network("reset".into()), DictationError::is_outage),
            (AsrError::Timeout, DictationError::is_outage),
        ] {
            let t = VendorTranscriber { client: Arc::new(Scripted { fail: Some(error.clone()), finishes: true, audio: Arc::clone(&audio), says: Says::Text }) };
            let failed = t.transcribe(b"wav", None, &[]).await.unwrap_err();
            assert!(check(&failed), "{error:?} → {failed:?}");
        }
    }
}
