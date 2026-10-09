//! Alibaba Cloud Model Studio as the core's ports (docs/dictation.md §3.4, §11.9): its
//! recognition models behind [`Transcriber`], and its realtime models behind
//! [`StreamingTranscriber`] too, so a take streams to the service while it is spoken and the
//! service's sentences are its text.
//!
//! A streaming session's WebSocket lives in a task on the core's runtime (the decode thread that
//! drives the session is a blocking task of it): `feed` queues 16-bit PCM for it, `poll` reads what
//! the service said, `finish` asks for the last sentence and waits for the task to end. A session
//! dropped without `finish` (a cancelled take) closes the connection; nothing more is sent.

use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;
use voltip_asr::duplex::{self, DuplexEvent, DuplexOptions};
use voltip_asr::{AsrConfig, DashscopeClient, DashscopeMode};
use voltip_core::dictation::{
    DictationError, LIVE_SAMPLE_RATE_HZ, Segment, StreamEvent, StreamFinal, StreamingSession, StreamingTranscriber, Transcriber, Transcript,
};

/// How long [`StreamingSession::finish`] waits for the service's last sentence: it answers within
/// a second of the last audio when the take was streamed as it was spoken.
pub const FINISH_TIMEOUT: Duration = Duration::from_secs(30);

/// A Model Studio model as the take's recogniser; a realtime one streams as well.
pub struct DashscopeTranscriber {
    client: DashscopeClient,
}

impl DashscopeTranscriber {
    /// Build the client for `mode`; fails on an unusable configuration (bad URL, empty model).
    pub fn new(config: AsrConfig, mode: DashscopeMode) -> Result<Self, DictationError> {
        Ok(Self { client: DashscopeClient::new(config, mode).map_err(crate::asr_error)? })
    }
}

#[async_trait]
impl Transcriber for DashscopeTranscriber {
    /// A whole take: the HTTP models in one request, a realtime model as a task it is sent to at
    /// once (the fallback when its stream did not run). The glossary goes out as hot words where
    /// the model takes them.
    async fn transcribe(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, DictationError> {
        let t = self.client.transcribe(wav, language, glossary).await.map_err(crate::asr_error)?;
        Ok(Transcript { text: t.text, latency_ms: t.latency_ms, model: Some(t.model) })
    }

    fn streaming(&self, glossary: &[String]) -> Option<Arc<dyn StreamingTranscriber>> {
        (self.client.mode() == DashscopeMode::Duplex).then(|| {
            Arc::new(DashscopeStreaming { client: self.client.clone(), glossary: glossary.to_vec(), finish_timeout: FINISH_TIMEOUT })
                as Arc<dyn StreamingTranscriber>
        })
    }
}

/// A realtime model's stream for one take, hinted with the take's glossary.
pub struct DashscopeStreaming {
    client: DashscopeClient,
    glossary: Vec<String>,
    finish_timeout: Duration,
}

impl StreamingTranscriber for DashscopeStreaming {
    /// Starts connecting in the background and returns at once: the audio fed meanwhile waits for
    /// the task to start, and a connection that fails reports through `poll`.
    fn open(&self, language: Option<&str>) -> Result<Box<dyn StreamingSession>, DictationError> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| DictationError::Asr("realtime model: no runtime for the connection".into()))?;
        let options = self.client.duplex_options(language, &self.glossary, LIVE_SAMPLE_RATE_HZ, true);
        let model = options.model.clone();
        let (commands, inbox) = mpsc::unbounded_channel();
        let (events_tx, events) = std_mpsc::channel();
        let (done_tx, done) = std_mpsc::sync_channel(1);
        runtime.spawn(async move {
            let result = drive(&options, inbox, &events_tx).await;
            if let Err(reason) = &result {
                let _ = events_tx.send(StreamEvent::Error(reason.clone()));
            }
            let _ = done_tx.send(result.map_err(DictationError::Asr));
        });
        Ok(Box::new(DashscopeSession { commands, events, done, finish_timeout: self.finish_timeout, model }))
    }

    /// Nothing to load: the model is the service's.
    fn warm(&self) {}
}

enum Command {
    Audio(Vec<u8>),
    Finish,
}

struct DashscopeSession {
    commands: mpsc::UnboundedSender<Command>,
    events: std_mpsc::Receiver<StreamEvent>,
    done: std_mpsc::Receiver<Result<StreamFinal, DictationError>>,
    finish_timeout: Duration,
    /// The realtime model the session streams to: its text's model (docs/dictation.md §3.5).
    model: String,
}

impl StreamingSession for DashscopeSession {
    fn feed(&mut self, pcm16k: &[f32]) {
        let pcm: Vec<u8> = pcm16k.iter().flat_map(|&x| ((x.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16).to_le_bytes()).collect();
        // A task that already ended has said why through `poll`.
        let _ = self.commands.send(Command::Audio(pcm));
    }

    fn poll(&mut self) -> StreamEvent {
        self.events.try_recv().unwrap_or(StreamEvent::Idle)
    }

    fn model(&self) -> Option<String> {
        Some(self.model.clone())
    }

    fn finish(self: Box<Self>) -> Result<StreamFinal, DictationError> {
        let _ = self.commands.send(Command::Finish);
        match self.done.recv_timeout(self.finish_timeout) {
            Ok(result) => result,
            Err(_) => Err(DictationError::Asr(format!("realtime model: no last sentence within {} s", self.finish_timeout.as_secs()))),
        }
    }
}

/// The task: connect, then pass audio on and results back until the service has said everything.
/// The error is the reason, as the preview reports it.
async fn drive(options: &DuplexOptions, mut inbox: mpsc::UnboundedReceiver<Command>, events: &std_mpsc::Sender<StreamEvent>) -> Result<StreamFinal, String> {
    let (mut sender, mut receiver) = duplex::connect(options).await.map_err(|e| e.to_string())?;
    let (mut committed, mut current) = (Vec::new(), String::new());
    let mut finishing = false;
    loop {
        tokio::select! {
            command = inbox.recv() => match command {
                Some(Command::Audio(pcm)) if !finishing => sender.audio(pcm).await.map_err(|e| e.to_string())?,
                Some(Command::Finish) if !finishing => {
                    finishing = true;
                    sender.finish().await.map_err(|e| e.to_string())?;
                }
                Some(_) => {}
                // The session was dropped: the take was cancelled (nothing more goes out), or
                // `finish` gave up waiting. Either way the connection closes now.
                None => {
                    sender.close().await;
                    return Err(if finishing { "realtime model: the last sentence came too late" } else { "realtime model: the take was cancelled" }.into());
                }
            },
            event = receiver.next() => match event.map_err(|e| e.to_string())? {
                DuplexEvent::Partial { text } => {
                    current.clone_from(&text);
                    let _ = events.send(StreamEvent::Partial { current: text });
                }
                DuplexEvent::Sentence { text, begin_ms, end_ms } => {
                    current.clear();
                    committed.push(Segment { text: text.clone(), start_ms: begin_ms, end_ms });
                    let _ = events.send(StreamEvent::Endpoint { text, start_ms: begin_ms, end_ms });
                }
                DuplexEvent::Finished => {
                    sender.close().await;
                    // A sentence the service never closed is the tail.
                    return Ok(StreamFinal { committed, tail: current, model: Some(options.model.clone()) });
                }
            },
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(crate) mod tests {
    use std::sync::Mutex;
    use std::time::Instant;

    use futures_util::{SinkExt, StreamExt};
    use serde_json::{Value, json};
    use tokio::net::{TcpListener, TcpStream};
    use tokio_tungstenite::tungstenite::Message;

    use super::*;

    /// What the scripted realtime endpoint saw.
    #[derive(Default)]
    pub(crate) struct Seen {
        pub run_task: Option<Value>,
        pub audio_bytes: usize,
        pub finished: bool,
        pub closed: bool,
    }

    /// A local realtime endpoint: a growing partial per 100 ms of audio, a sentence per 300 ms, the
    /// rest as one sentence at `finish-task` (or `fail` at `run-task`). Returns its http origin.
    pub(crate) async fn serve(fail: Option<&'static str>) -> (String, Arc<Mutex<Seen>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Seen::default()));
        let shared = seen.clone();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                tokio::spawn(session(tcp, fail, shared.clone()));
            }
        });
        (origin, seen)
    }

    async fn session(tcp: TcpStream, fail: Option<&'static str>, seen: Arc<Mutex<Seen>>) {
        let Ok(socket) = tokio_tungstenite::accept_async(tcp).await else { return };
        let (mut sink, mut stream) = socket.split();
        let event = |name: &str, payload: Value| Message::text(json!({ "header": { "task_id": "t", "event": name }, "payload": payload }).to_string());
        let sentence = |id: usize, text: String, end: bool, begin: usize| {
            event(
                "result-generated",
                json!({ "output": { "sentence": { "sentence_id": id, "text": text, "sentence_end": end, "begin_time": begin, "end_time": begin + 300 } } }),
            )
        };
        let (mut frames, mut words) = (0usize, 0usize);
        while let Some(message) = stream.next().await {
            match message {
                Ok(Message::Text(text)) => {
                    let value: Value = serde_json::from_str(text.as_str()).unwrap();
                    match value.pointer("/header/action").and_then(Value::as_str) {
                        Some("run-task") => {
                            seen.lock().unwrap().run_task = Some(value.clone());
                            if let Some(code) = fail {
                                let failed = json!({ "header": { "event": "task-failed", "error_code": code, "error_message": "scripted" }, "payload": {} });
                                let _ = sink.send(Message::text(failed.to_string())).await;
                                return;
                            }
                            let _ = sink.send(event("task-started", json!({}))).await;
                        }
                        Some("finish-task") => {
                            seen.lock().unwrap().finished = true;
                            if words > 0 {
                                let _ = sink.send(sentence(frames / 3 + 1, format!("{}。", "字".repeat(words)), true, frames / 3 * 300)).await;
                            }
                            let _ = sink.send(event("task-finished", json!({}))).await;
                        }
                        _ => {}
                    }
                }
                Ok(Message::Binary(bytes)) => {
                    seen.lock().unwrap().audio_bytes += bytes.len();
                    frames += 1;
                    words += 1;
                    let id = (frames - 1) / 3 + 1;
                    if frames % 3 == 0 {
                        let _ = sink.send(sentence(id, format!("第{id}句。"), true, (id - 1) * 300)).await;
                        words = 0;
                    } else {
                        let _ = sink.send(sentence(id, "字".repeat(words), false, (id - 1) * 300)).await;
                    }
                }
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(_) => {}
            }
        }
        seen.lock().unwrap().closed = true;
    }

    pub(crate) fn transcriber(origin: &str, model: &str, mode: DashscopeMode) -> DashscopeTranscriber {
        DashscopeTranscriber::new(AsrConfig::new(format!("{origin}/compatible-mode/v1"), model).with_token(Some("sk-test".into())), mode).unwrap()
    }

    /// Feed `chunks` of 100 ms on a blocking thread, polling after each like the decode thread
    /// does, until `want` events came; then finish.
    fn drive_session(streaming: Arc<dyn StreamingTranscriber>, chunks: usize, want: usize) -> (Vec<StreamEvent>, Result<StreamFinal, DictationError>) {
        let mut session = streaming.open(Some("zh")).unwrap();
        // docs/dictation.md §3.5: the session names the model its text comes from.
        assert!(session.model().is_some_and(|m| m.starts_with("qwen-audio-")), "{:?}", session.model());
        let mut events = Vec::new();
        for _ in 0..chunks {
            session.feed(&[0.25; 1600]);
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while events.len() < want && Instant::now() < deadline {
            match session.poll() {
                StreamEvent::Idle => std::thread::sleep(Duration::from_millis(5)),
                event => events.push(event),
            }
        }
        (events, session.finish())
    }

    /// docs/dictation.md §11.9: the take's audio streams to the realtime model, its partials and
    /// sentences come back through `poll`, and `finish` ends the task with every sentence.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_realtime_model_streams_partials_and_sentences() {
        let (origin, seen) = serve(None).await;
        let t = transcriber(&origin, "qwen-audio-3.1-asr-flash-message", DashscopeMode::Duplex);
        let streaming = t.streaming(&["Voltip".into()]).expect("a realtime model streams");
        streaming.warm();
        // Five chunks: partial, partial, sentence 1, partial, partial; the tail closes at finish.
        let (events, fin) = tokio::task::spawn_blocking(move || drive_session(streaming, 5, 5)).await.unwrap();
        assert_eq!(
            events,
            vec![
                StreamEvent::Partial { current: "字".into() },
                StreamEvent::Partial { current: "字字".into() },
                StreamEvent::Endpoint { text: "第1句。".into(), start_ms: 0, end_ms: 300 },
                StreamEvent::Partial { current: "字".into() },
                StreamEvent::Partial { current: "字字".into() },
            ]
        );
        let fin = fin.unwrap();
        assert_eq!(fin.committed.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), ["第1句。", "字字。"]);
        assert_eq!(fin.tail, "");
        assert_eq!(fin.model.as_deref(), Some("qwen-audio-3.1-asr-flash-message"), "the receipt comes back with the flush");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.audio_bytes, 5 * 3200, "16-bit PCM, 100 ms per feed");
        assert!(seen.finished);
        let parameters = seen.run_task.as_ref().and_then(|r| r.pointer("/payload/parameters")).cloned().unwrap();
        assert_eq!(parameters["intermediate_result_enabled"], json!(true), "a live take asks for partials: {parameters}");
        assert_eq!(parameters["vocabulary"], json!({ "Voltip": 4 }));
        assert_eq!(parameters["sample_rate"], json!(16000));
        // The HTTP models take whole recordings only.
        assert!(transcriber(&origin, "qwen-audio-3.1-asr-flash", DashscopeMode::Multimodal).streaming(&[]).is_none());
        assert!(transcriber(&origin, "qwen3-asr-flash", DashscopeMode::Chat).streaming(&[]).is_none());
    }

    /// A task the service fails reports the reason through `poll` and `finish`; a session dropped
    /// mid-take closes the connection without `finish-task`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_or_cancelled_stream_ends_cleanly() {
        let (origin, _) = serve(Some("AllocationQuota.FreeTierOnly")).await;
        let streaming = transcriber(&origin, "qwen-audio-3.1-asr-flash-streaming", DashscopeMode::Duplex).streaming(&[]).unwrap();
        let (events, fin) = tokio::task::spawn_blocking(move || drive_session(streaming, 1, 1)).await.unwrap();
        assert!(matches!(&events[..], [StreamEvent::Error(m)] if m.contains("quota used up")), "{events:?}");
        assert!(matches!(&fin, Err(DictationError::Asr(m)) if m.contains("quota used up")), "{fin:?}");
        // Cancelled: the session is dropped after some audio.
        let (origin, seen) = serve(None).await;
        let streaming = transcriber(&origin, "qwen-audio-3.1-asr-flash-streaming", DashscopeMode::Duplex).streaming(&[]).unwrap();
        tokio::task::spawn_blocking(move || {
            let mut session = streaming.open(None).unwrap();
            session.feed(&[0.1; 1600]);
            let deadline = Instant::now() + Duration::from_secs(20);
            while session.poll() == StreamEvent::Idle && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            drop(session);
        })
        .await
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !seen.lock().unwrap().closed && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let seen = seen.lock().unwrap();
        assert!(seen.closed && !seen.finished, "closed without finish-task");
        assert_eq!(seen.audio_bytes, 3200);
    }

    /// Nobody answers the last sentence: `finish` gives up after its timeout instead of hanging
    /// the take.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn finish_gives_up_on_a_silent_service() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        // Accepts the WebSocket, starts the task and then says nothing more.
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let (mut sink, mut stream) = socket.split();
            let _ = stream.next().await;
            let _ = sink.send(Message::text(json!({ "header": { "event": "task-started" }, "payload": {} }).to_string())).await;
            while stream.next().await.is_some() {}
        });
        let t = transcriber(&origin, "fun-asr-realtime", DashscopeMode::Duplex);
        let streaming = Arc::new(DashscopeStreaming { client: t.client.clone(), glossary: Vec::new(), finish_timeout: Duration::from_millis(300) });
        let fin = tokio::task::spawn_blocking(move || {
            let mut session = streaming.open(None).unwrap();
            session.feed(&[0.0; 1600]);
            session.finish()
        })
        .await
        .unwrap();
        assert!(matches!(&fin, Err(DictationError::Asr(m)) if m.contains("no last sentence")), "{fin:?}");
        // The session gave up: its task closes the connection, so the server's read ends.
        tokio::time::timeout(Duration::from_secs(20), server).await.expect("the connection is closed").unwrap();
    }

    #[test]
    fn opening_needs_a_runtime() {
        let t = transcriber("http://127.0.0.1:9", "fun-asr-realtime", DashscopeMode::Duplex);
        let err = t.streaming(&[]).unwrap().open(None).err().expect("no runtime here");
        assert!(matches!(&err, DictationError::Asr(m) if m.contains("no runtime")), "{err:?}");
        assert!(DashscopeTranscriber::new(AsrConfig::new("nope", "m"), DashscopeMode::Chat).is_err());
    }
}
