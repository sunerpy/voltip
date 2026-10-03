#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Alibaba Cloud Model Studio's recognition against the real service (docs/dictation.md §3.4,
//! §11.9): a realtime model fed a 16 kHz mono WAV at the pace a microphone delivers it, the same
//! take sent whole, and a whole-file model over HTTP. Ignored; it reaches the network and needs a
//! key. Use models the account has free quota for (百炼控制台 › 免费额度), ideally with
//! 「免费额度用完即停」 on, so a run never costs money:
//!
//! ```text
//! VOLTIP_REAL_DASHSCOPE_URL=https://<workspace>.cn-beijing.maas.aliyuncs.com/compatible-mode/v1 \
//!   VOLTIP_REAL_DASHSCOPE_KEY=<key> VOLTIP_LOCAL_SAMPLE_WAV=/tmp/voltip-sample.wav \
//!   cargo test -p voltip-cloud --test real_dashscope -- --ignored --nocapture --test-threads=1
//! ```
//!
//! `VOLTIP_REAL_DASHSCOPE_STREAMING` (default `qwen-audio-3.1-asr-flash-streaming`,
//! `qwen-audio-3.1-asr-flash-message`, comma-separated) and `VOLTIP_REAL_DASHSCOPE_FILE` (default
//! `qwen-audio-3.1-asr-flash`) pick the models. The sample is the 16 s Chinese take the other real
//! tests use (「我想创建一个 good idea 吧……集成在 Teams 里面……」).

use std::sync::Arc;
use std::time::{Duration, Instant};

use voltip_core::dictation::wav;
use voltip_core::dictation::{LIVE_CHUNK_SAMPLES, StreamEvent, Transcriber};
use voltip_core::{AsrProtocol, RemoteService};

struct Real {
    url: String,
    key: String,
    wav: Vec<u8>,
    samples: Vec<f32>,
}

fn real() -> Real {
    let url = std::env::var("VOLTIP_REAL_DASHSCOPE_URL").expect("VOLTIP_REAL_DASHSCOPE_URL");
    let key = std::env::var("VOLTIP_REAL_DASHSCOPE_KEY").expect("VOLTIP_REAL_DASHSCOPE_KEY");
    let sample = std::env::var("VOLTIP_LOCAL_SAMPLE_WAV").unwrap_or_else(|_| "/tmp/voltip-sample.wav".into());
    let wav = std::fs::read(&sample).expect("sample wav");
    let pcm = wav::pcm_data(&wav).expect("a 16-bit PCM WAV");
    let samples = pcm.as_chunks::<2>().0.iter().map(|b| f32::from(i16::from_le_bytes(*b)) / f32::from(i16::MAX)).collect();
    Real { url, key, wav, samples }
}

fn models(var: &str, default: &str) -> Vec<String> {
    std::env::var(var).unwrap_or_else(|_| default.to_owned()).split(',').map(|m| m.trim().to_owned()).filter(|m| !m.is_empty()).collect()
}

fn transcriber(real: &Real, model: &str) -> Arc<dyn Transcriber> {
    let remote = RemoteService { url: real.url.clone(), model: model.to_owned(), key: Some(real.key.clone()) };
    voltip_cloud::transcriber_for(AsrProtocol::of(&real.url, model), &remote)
}

/// The words of the sample any decent recognition gets, whatever its casing and spacing.
fn assert_sample_text(model: &str, text: &str) {
    let flat: String = text.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    assert!(flat.contains("goodidea") && flat.contains("teams") && flat.contains("创建"), "{model}: {text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "reaches Model Studio: VOLTIP_REAL_DASHSCOPE_URL, VOLTIP_REAL_DASHSCOPE_KEY and VOLTIP_LOCAL_SAMPLE_WAV"]
async fn real_realtime_models_stream_a_spoken_take() {
    let real = real();
    for model in models("VOLTIP_REAL_DASHSCOPE_STREAMING", "qwen-audio-3.1-asr-flash-streaming,qwen-audio-3.1-asr-flash-message") {
        assert!(AsrProtocol::of(&real.url, &model).streams(), "{model} is not a realtime model");
        let streaming = transcriber(&real, &model).streaming(&["Teams".into(), "good idea".into()]).expect("a realtime model streams");
        let samples = real.samples.clone();
        let (events, fin, total, flush) = tokio::task::spawn_blocking(move || {
            let mut session = streaming.open(Some("zh")).unwrap();
            let started = Instant::now();
            let mut events = Vec::new();
            // At the microphone's pace: 100 ms of audio every 100 ms, polled after each.
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
            let flushed = Instant::now();
            let fin = session.finish();
            (events, fin, started.elapsed(), flushed.elapsed())
        })
        .await
        .unwrap();
        let fin = fin.unwrap_or_else(|e| panic!("{model}: {e}"));
        let text = voltip_asr_join(&fin.committed.iter().map(|s| s.text.clone()).chain(std::iter::once(fin.tail.clone())).collect::<Vec<_>>());
        println!("{model}: {} events, first at {:?}, finish took {flush:?} (total {total:?})", events.len(), events.first().map(|(t, _)| *t));
        for (at, event) in events.iter().filter(|(_, e)| !matches!(e, StreamEvent::Partial { .. })) {
            println!("  {at:?} {event:?}");
        }
        println!("  final: {text}");
        assert!(events.iter().all(|(_, e)| !matches!(e, StreamEvent::Error(_))), "{model}: {events:?}");
        assert!(events.iter().any(|(_, e)| matches!(e, StreamEvent::Partial { .. })), "{model}: partials while speaking");
        assert!(flush < Duration::from_secs(5), "{model}: the last sentence came {flush:?} after the take");
        assert_sample_text(&model, &text);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "reaches Model Studio: VOLTIP_REAL_DASHSCOPE_URL, VOLTIP_REAL_DASHSCOPE_KEY and VOLTIP_LOCAL_SAMPLE_WAV"]
async fn real_whole_takes_on_every_protocol() {
    let real = real();
    let streaming = models("VOLTIP_REAL_DASHSCOPE_STREAMING", "qwen-audio-3.1-asr-flash-streaming");
    for model in models("VOLTIP_REAL_DASHSCOPE_FILE", "qwen-audio-3.1-asr-flash").into_iter().chain(streaming.into_iter().take(1)) {
        let started = Instant::now();
        let t = transcriber(&real, &model).transcribe(&real.wav, Some("zh"), &["Teams".into(), "good idea".into()]).await;
        let t = t.unwrap_or_else(|e| panic!("{model}: {e}"));
        println!("{model} ({:?}): {:?} → {}", AsrProtocol::of(&real.url, &model), started.elapsed(), t.text);
        assert_sample_text(&model, &t.text);
    }
}

/// The sentences joined the way the take's text is (no space between Chinese sentences).
fn voltip_asr_join(parts: &[String]) -> String {
    parts.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("")
}
