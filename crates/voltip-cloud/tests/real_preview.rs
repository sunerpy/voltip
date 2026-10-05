#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The built-in service's live preview against a real service (docs/dictation.md §11.8):
//! `RedecodeStreaming` over the real cloud client, fed a 16 kHz mono WAV at the pace a microphone
//! delivers it. Ignored; it reaches the network and needs a service:
//!
//! ```text
//! VOLTIP_REAL_ASR_URL=https://<asr-host> VOLTIP_REAL_ASR_TOKEN=<token> VOLTIP_REAL_ASR_MODEL=Qwen/Qwen3-ASR-1.7B \
//!   VOLTIP_LOCAL_SAMPLE_WAV=/tmp/voltip-sample.wav \
//!   cargo test -p voltip-cloud --test real_preview -- --ignored --nocapture
//! ```
//!
//! Measured 2026-10-02 (the 16 s Chinese sample, through the built-in service's edge): speech from
//! 0.9 s, the first preview at 3.4 s, then 4.8, 7.7, 10.7, 12.6 and 15.1 s; one sentence (no 0.8 s
//! pause), its final decode 1.9 s (docs/dictation.md §11.8).

use std::sync::Arc;
use std::time::{Duration, Instant};

use voltip_core::dictation::redecode::RedecodeStreaming;
use voltip_core::dictation::wav;
use voltip_core::dictation::{LIVE_CHUNK_SAMPLES, StreamEvent, StreamingTranscriber};
use voltip_core::{ProviderId, ResolvedEngines};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "reaches a real ASR service: VOLTIP_REAL_ASR_URL, VOLTIP_REAL_ASR_TOKEN, VOLTIP_REAL_ASR_MODEL and VOLTIP_LOCAL_SAMPLE_WAV"]
async fn real_service_previews_a_spoken_sample() {
    let url = std::env::var("VOLTIP_REAL_ASR_URL").expect("VOLTIP_REAL_ASR_URL");
    let token = std::env::var("VOLTIP_REAL_ASR_TOKEN").ok();
    let model = std::env::var("VOLTIP_REAL_ASR_MODEL").unwrap_or_else(|_| "Qwen/Qwen3-ASR-1.7B".into());
    let sample = std::env::var("VOLTIP_LOCAL_SAMPLE_WAV").unwrap_or_else(|_| "/tmp/voltip-sample.wav".into());
    let bytes = std::fs::read(&sample).expect("sample wav");
    let pcm = wav::pcm_data(&bytes).expect("a 16-bit PCM WAV");
    let samples: Vec<f32> = pcm.as_chunks::<2>().0.iter().map(|b| f32::from(i16::from_le_bytes(*b)) / f32::from(i16::MAX)).collect();
    let mut engines = ResolvedEngines::resolve(&Default::default(), &Default::default(), &voltip_core::BuiltIn::EMPTY);
    engines.asr_provider = ProviderId::Custom;
    engines.asr_remote = Some(voltip_core::RemoteService { url, model, key: token, ..voltip_core::RemoteService::default() });
    let transcriber = voltip_cloud::remote_transcriber(&engines);
    let streaming = Arc::new(RedecodeStreaming::new(transcriber, Vec::new()).flushing(true));
    let (events, fin, total) = tokio::task::spawn_blocking(move || {
        let mut session = streaming.open(Some("zh")).unwrap();
        let started = Instant::now();
        let mut events = Vec::new();
        // At the microphone's pace: 100 ms of audio every 100 ms, polled after each, as `run_live` does.
        for (i, chunk) in samples.chunks(LIVE_CHUNK_SAMPLES).enumerate() {
            let due = Duration::from_millis(100 * u64::try_from(i).unwrap());
            if let Some(wait) = due.checked_sub(started.elapsed()) {
                std::thread::sleep(wait);
            }
            session.feed(chunk);
            loop {
                match session.poll() {
                    StreamEvent::Idle => break,
                    event => events.push((started.elapsed(), event)),
                }
            }
        }
        let flush = Instant::now();
        let fin = session.finish().unwrap();
        (events, fin, (started.elapsed(), flush.elapsed()))
    })
    .await
    .unwrap();
    for (at, event) in &events {
        println!("{:>6} ms  {event:?}", at.as_millis());
    }
    println!("flush {} ms; total {} ms; final {fin:?}", total.1.as_millis(), total.0.as_millis());
    assert!(events.iter().any(|(_, e)| matches!(e, StreamEvent::Partial { .. })), "a preview came back while the sample played");
    assert!(events.iter().all(|(_, e)| !matches!(e, StreamEvent::Error(_))), "{events:?}");
    let text: String = fin.committed.iter().map(|s| s.text.as_str()).chain(std::iter::once(fin.tail.as_str())).collect();
    assert!(text.contains("创建"), "the sample says 想创建: {text}");
}
