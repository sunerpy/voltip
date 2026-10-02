#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Real inference through the local engines. Each test needs a downloaded model and a 16 kHz mono
//! WAV (`VOLTIP_LOCAL_SAMPLE_WAV`, default `/tmp/voltip-sample.wav`):
//!
//! ```text
//! VOLTIP_LOCAL_MODEL_DIR=/path/to/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17 \
//! VOLTIP_LOCAL_GGUF=/path/to/Qwen3-ASR-0.6B-Q6_K.gguf \
//! VOLTIP_LOCAL_STREAM_DIR=/path/to/sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05 \
//! VOLTIP_LOCAL_VAD_MODEL=/path/to/silero_vad.onnx \
//!   cargo test -p voltip-asr-local --test real -- --ignored --nocapture
//! ```
//!
//! On a GPU host, `--features vulkan` (Linux / Windows, Vulkan SDK at build time) or macOS (Metal)
//! makes `real_gguf_runs_where_the_compute_choice_says` run the GGUF tier on the GPU as well.
//!
//! `VOLTIP_LOCAL_MODEL_DIR` is a directory holding `model.int8.onnx` + `tokens.txt`; the catalogue
//! entry is picked by the directory name (`paraformer` → `paraformer-zh`, otherwise
//! `sense-voice-small`). `VOLTIP_LOCAL_GGUF` is the 0.6B (or 1.7B) Q6_K file; `VOLTIP_LOCAL_STREAM_DIR`
//! the streaming Zipformer's five files. Each test stages the files into a temporary library with
//! a manifest so the installation check passes, then prints latencies and text.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use voltip_asr_local::{
    CATALOGUE_VERSION, Compute, LocalDevice, LocalStreamingTranscriber, LocalTranscriber, MANIFEST_FILE, Manifest, ModelEntry, VadSegmenterFactory, VadTrimmer,
    entry, hardware, streaming_entry, vad_entry,
};
use voltip_core::dictation::{LIVE_CHUNK_SAMPLES, SegmenterFactory, StreamEvent, StreamingTranscriber, Transcriber};

/// Stage `entry`'s files from `sources` (file name → path) into `root/<id>` with a manifest, as the
/// store would have installed them; the sizes are checked against the catalogue.
fn stage(root: &Path, e: &ModelEntry, source_of: impl Fn(&str) -> PathBuf) {
    let dir = root.join(e.id);
    std::fs::create_dir_all(&dir).unwrap();
    for f in e.files() {
        let src = source_of(f.name);
        assert!(src.exists(), "{} missing", src.display());
        std::fs::hard_link(&src, dir.join(f.name)).or_else(|_| std::fs::copy(&src, dir.join(f.name)).map(|_| ())).unwrap();
        assert_eq!(std::fs::metadata(dir.join(f.name)).unwrap().len(), f.size, "{}: size differs from the catalogue", f.name);
    }
    let manifest = Manifest {
        id: e.id.into(),
        version: CATALOGUE_VERSION,
        downloaded_at: 1,
        files: e.files().iter().map(|f| (f.name.to_owned(), f.sha256.to_owned())).collect(),
    };
    std::fs::write(dir.join(MANIFEST_FILE), serde_json::to_vec(&manifest).unwrap()).unwrap();
}

fn sample_wav() -> Vec<u8> {
    let sample = PathBuf::from(std::env::var("VOLTIP_LOCAL_SAMPLE_WAV").unwrap_or_else(|_| "/tmp/voltip-sample.wav".into()));
    std::fs::read(&sample).expect("sample wav")
}

fn is_chinese(text: &str) -> bool {
    text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

#[tokio::test]
#[ignore = "needs VOLTIP_LOCAL_MODEL_DIR (a downloaded sherpa-onnx model) and VOLTIP_LOCAL_SAMPLE_WAV"]
async fn real_model_transcribes_the_sample() {
    let model_dir = PathBuf::from(std::env::var("VOLTIP_LOCAL_MODEL_DIR").expect("VOLTIP_LOCAL_MODEL_DIR"));
    let id = if model_dir.to_string_lossy().contains("paraformer") { "paraformer-zh" } else { "sense-voice-small" };
    let e = entry(id).unwrap();
    // Stage the model as the store would have installed it (files + manifest); the hashes are the
    // catalogue's, so a wrong download would show up as "not installed".
    let root = tempfile::tempdir().unwrap();
    stage(root.path(), e, |name| model_dir.join(name));
    let wav = sample_wav();
    let t = LocalTranscriber::new(root.path()).select(e.id);
    let cold = Instant::now();
    let first = t.transcribe(&wav, Some("zh"), &[]).await.unwrap();
    let cold_ms = cold.elapsed().as_millis();
    let warm = Instant::now();
    let second = t.transcribe(&wav, Some("zh"), &[]).await.unwrap();
    let warm_ms = warm.elapsed().as_millis();
    println!("model={id} cold(load+infer)={cold_ms} ms warm(infer)={warm_ms} ms latency_ms={} text={}", second.latency_ms, second.text);
    assert!(!first.text.is_empty(), "empty transcript");
    assert_eq!(first.text, second.text, "deterministic");
    assert!(is_chinese(&first.text), "expected Chinese text, got {}", first.text);
    assert_eq!(t.loaded().as_deref(), Some(e.id));
    assert!(warm_ms < cold_ms, "the second run reuses the loaded model");
}

/// transcribe.cpp over the Qwen3-ASR GGUF (docs/dictation.md §10, the default tier). The entry is
/// picked by the file name (`1.7B` → `qwen3-asr-1.7b`, otherwise `qwen3-asr-0.6b`). Measured
/// 2026-09-25 on this host: 0.6B Q6_K load 0.64 s, 16 s Chinese sample 1.56 s.
#[tokio::test]
#[ignore = "needs VOLTIP_LOCAL_GGUF (a downloaded Qwen3-ASR Q6_K gguf) and VOLTIP_LOCAL_SAMPLE_WAV"]
async fn real_qwen3_gguf_transcribes_the_sample() {
    let gguf = PathBuf::from(std::env::var("VOLTIP_LOCAL_GGUF").expect("VOLTIP_LOCAL_GGUF"));
    let id = if gguf.to_string_lossy().contains("1.7B") { "qwen3-asr-1.7b" } else { "qwen3-asr-0.6b" };
    let e = entry(id).unwrap();
    let root = tempfile::tempdir().unwrap();
    stage(root.path(), e, |_| gguf.clone());
    let wav = sample_wav();
    let t = LocalTranscriber::new(root.path()).select(e.id);
    let cold = Instant::now();
    // The language hint is dropped for this family (unsupported upstream); `Some("zh")` must not fail.
    let first = t.transcribe(&wav, Some("zh"), &[]).await.unwrap();
    let cold_ms = cold.elapsed().as_millis();
    let warm = Instant::now();
    let second = t.transcribe(&wav, None, &[]).await.unwrap();
    let warm_ms = warm.elapsed().as_millis();
    println!("model={id} cold(load+infer)={cold_ms} ms warm(infer)={warm_ms} ms latency_ms={} text={}", second.latency_ms, second.text);
    assert!(!first.text.is_empty(), "empty transcript");
    assert_eq!(first.text, second.text, "deterministic");
    assert!(is_chinese(&first.text), "expected Chinese text, got {}", first.text);
    assert!(first.text.contains("想创建"), "the sample says 想创建: {}", first.text);
    assert_eq!(t.loaded().as_deref(), Some(e.id));
    assert!(warm_ms < cold_ms, "the second run reuses the loaded model");
}

/// The streaming Zipformer (docs/dictation.md §11) fed the sample in 100 ms chunks like the live
/// tap does: several distinct partials, at least one endpoint before the end, the flushed text
/// contains the sample's words. Measured 2026-09-25: load 2.6 s, 4 ms avg / 44 ms max per chunk.
#[test]
#[ignore = "needs VOLTIP_LOCAL_STREAM_DIR (the downloaded streaming Zipformer directory) and VOLTIP_LOCAL_SAMPLE_WAV"]
fn real_streaming_model_emits_partials() {
    let stream_dir = PathBuf::from(std::env::var("VOLTIP_LOCAL_STREAM_DIR").expect("VOLTIP_LOCAL_STREAM_DIR"));
    let e = streaming_entry();
    let root = tempfile::tempdir().unwrap();
    stage(root.path(), e, |name| stream_dir.join(name));
    let (samples, rate) = voltip_asr_local::decode_wav(&sample_wav()).unwrap();
    assert_eq!(rate, 16_000, "the sample must be 16 kHz mono");
    let t = LocalStreamingTranscriber::new(root.path());
    assert!(t.is_installed());
    let load = Instant::now();
    let mut session = t.open(Some("zh")).unwrap();
    let load_ms = load.elapsed().as_millis();
    assert!(t.is_loaded());
    let mut partials: Vec<String> = Vec::new();
    let mut endpoints = 0;
    let (mut total_us, mut max_us, mut chunks) = (0u128, 0u128, 0u32);
    for chunk in samples.chunks(LIVE_CHUNK_SAMPLES) {
        let step = Instant::now();
        session.feed(chunk);
        loop {
            match session.poll() {
                StreamEvent::Idle => break,
                StreamEvent::Error(e) => panic!("decoder error: {e}"),
                StreamEvent::Partial { current } => {
                    if partials.last() != Some(&current) {
                        partials.push(current);
                    }
                }
                StreamEvent::Endpoint { text, start_ms, end_ms } => {
                    println!("endpoint {start_ms}..{end_ms} ms: {text}");
                    endpoints += 1;
                }
            }
        }
        let us = step.elapsed().as_micros();
        total_us += us;
        max_us = max_us.max(us);
        chunks += 1;
    }
    let fin = session.finish().unwrap();
    let text: String = fin.committed.iter().map(|s| s.text.as_str()).chain(std::iter::once(fin.tail.as_str())).collect();
    println!(
        "load={load_ms} ms chunks={chunks} avg={} us max={} us partials={} endpoints={endpoints} committed={} tail={:?} text={text}",
        total_us / u128::from(chunks.max(1)),
        max_us,
        partials.len(),
        fin.committed.len(),
        fin.tail
    );
    let distinct: std::collections::BTreeSet<&String> = partials.iter().collect();
    assert!(distinct.len() >= 5, "expected ≥ 5 distinct partials, got {}: {partials:?}", distinct.len());
    assert!(text.contains("想创建"), "the sample says 想创建: {text}");
    assert!(is_chinese(&text));
    // Second session on the same loaded recogniser: no reload.
    let again = Instant::now();
    let s2 = t.open(None).unwrap();
    assert!(again.elapsed().as_millis() < load_ms.max(50), "the recogniser stays loaded");
    assert!(s2.finish().unwrap().tail.is_empty());
}

/// The Silero VAD through sherpa-onnx (docs/dictation.md §12 `vad_trim`): a synthetic take of
/// 1.5 s silence + 1 s of speech-like noise bursts + 1.5 s silence is cut to the bursts plus 450 ms
/// on each side; a silent take is left alone (fail-open); the sample WAV, when present, is trimmed
/// without losing its speech. `VOLTIP_LOCAL_VAD_MODEL` points at `silero_vad.onnx` (HF
/// `csukuangfj/vad`, 1 807 522 B).
#[tokio::test]
#[ignore = "needs VOLTIP_LOCAL_VAD_MODEL (the downloaded silero_vad.onnx)"]
async fn real_vad_trims_silence_around_speech() {
    let model = PathBuf::from(std::env::var("VOLTIP_LOCAL_VAD_MODEL").expect("VOLTIP_LOCAL_VAD_MODEL"));
    let e = vad_entry();
    let root = tempfile::tempdir().unwrap();
    stage(root.path(), e, |_| model.clone());
    let trimmer = VadTrimmer::new(root.path());
    assert!(trimmer.is_installed());
    // Speech-like signal: amplitude-modulated noise bursts at syllable rate (Silero ignores pure tones).
    let mut x: u64 = 42;
    let mut noise = || {
        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((x >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    };
    let silence = 1500 * 16;
    let speech = 1000 * 16;
    let mut take = vec![0.0_f32; silence];
    for i in 0..speech {
        let t = i as f32 / 16_000.0;
        let envelope = 0.5 + 0.5 * (t * 4.0 * std::f32::consts::TAU).sin();
        let voiced = (t * 140.0 * std::f32::consts::TAU).sin() * 0.3 + (t * 700.0 * std::f32::consts::TAU).sin() * 0.2;
        take.push((voiced + noise() * 0.15) * envelope);
    }
    take.extend(std::iter::repeat_n(0.0_f32, silence));
    let cold = Instant::now();
    let t = trimmer.trim(&take, 16_000);
    let cold_ms = cold.elapsed().as_millis();
    let warm = Instant::now();
    let again = trimmer.trim(&take, 16_000);
    let warm_ms = warm.elapsed().as_millis();
    println!(
        "vad cold(load+detect)={cold_ms} ms warm={warm_ms} ms in={} ms out={} ms cut_start={} ms cut_end={} ms skipped={:?}",
        take.len() / 16,
        t.samples.len() / 16,
        t.cut_start / 16,
        t.cut_end / 16,
        t.skipped
    );
    assert!(t.applied(), "{:?}", t.skipped);
    assert_eq!(t, again, "deterministic");
    let out_ms = t.samples.len() / 16;
    assert!((1700..=2400).contains(&out_ms), "≈ 1 s of speech + 2 × 450 ms padding, got {out_ms} ms");
    // The detector needs `min_speech` to open and `min_silence` to close a segment, so the cut is a
    // few hundred milliseconds inside the 1.5 s of silence on each side.
    assert!(t.cut_start / 16 >= 800 && t.cut_end / 16 >= 500, "most of the silence goes on each side: {} / {} ms", t.cut_start / 16, t.cut_end / 16);
    let silent = vec![0.0_f32; 3 * 16_000];
    let s = trimmer.trim(&silent, 16_000);
    assert_eq!(s.samples, silent);
    assert_eq!(s.skipped.as_deref(), Some("VAD found no speech"));
    if let Ok(wav) = std::fs::read(std::env::var("VOLTIP_LOCAL_SAMPLE_WAV").unwrap_or_else(|_| "/tmp/voltip-sample.wav".into())) {
        let (samples, rate) = voltip_asr_local::decode_wav(&wav).unwrap();
        let t = trimmer.trim(&samples, rate);
        println!(
            "sample: in={} ms out={} ms cut_start={} ms cut_end={} ms",
            samples.len() as u32 / (rate / 1000),
            t.samples.len() as u32 / (rate / 1000),
            t.cut_start as u32 / (rate / 1000),
            t.cut_end as u32 / (rate / 1000)
        );
        assert!(t.applied());
        assert!(t.samples.len() * 10 >= samples.len() * 5, "a spoken sample keeps at least half of its length");
    }
}

/// A long take cut where it pauses (docs/dictation.md §22): the sample WAV repeated for three
/// minutes with 0.8 s of silence between the repeats, fed to the desktop's segmenter in the
/// recording thread's chunks. Every segment is 20–45 s long, and every cut lands in a pause.
#[test]
#[ignore = "needs VOLTIP_LOCAL_VAD_MODEL (the downloaded silero_vad.onnx) and VOLTIP_LOCAL_SAMPLE_WAV"]
fn real_vad_cuts_a_long_take_at_its_pauses() {
    let model = PathBuf::from(std::env::var("VOLTIP_LOCAL_VAD_MODEL").expect("VOLTIP_LOCAL_VAD_MODEL"));
    let root = tempfile::tempdir().unwrap();
    stage(root.path(), vad_entry(), |_| model.clone());
    let (sample, rate) = voltip_asr_local::decode_wav(&sample_wav()).unwrap();
    assert_eq!(rate, 16_000, "the sample is 16 kHz mono");
    let mut take = Vec::new();
    while take.len() < 180 * 16_000 {
        take.extend_from_slice(&sample);
        take.extend(std::iter::repeat_n(0.0_f32, 800 * 16));
    }
    let mut segmenter = VadSegmenterFactory::new(root.path(), None).create().unwrap();
    let started = Instant::now();
    let mut cuts = Vec::new();
    for chunk in take.chunks(4096) {
        cuts.extend(segmenter.push(chunk));
    }
    let end = segmenter.finish().unwrap();
    let elapsed = started.elapsed();
    let rms = |a: &[f32]| (a.iter().map(|s| s * s).sum::<f32>() / a.len().max(1) as f32).sqrt();
    let overall = rms(&take);
    let seconds: Vec<f64> = cuts.iter().map(|&c| c as f64 / 16_000.0).collect();
    println!("vad segmenter: {} s in {elapsed:?}, cuts at {seconds:.2?}, end {} s", take.len() / 16_000, end / 16_000);
    assert_eq!(end, take.len() as u64);
    assert!(!cuts.is_empty());
    let mut start = 0;
    for &cut in &cuts {
        let len = (cut - start) as f64 / 16_000.0;
        assert!((20.0..=45.0).contains(&len), "a segment of {len:.2} s ending at {:.2} s", cut as f64 / 16_000.0);
        let at = usize::try_from(cut).unwrap();
        let around = &take[at.saturating_sub(400)..(at + 400).min(take.len())];
        // The sample's own pauses carry its room noise (about −45 dBFS); speech is about −30.
        assert!(rms(around) < 0.01, "the cut at {:.2} s is not in a pause (RMS {} against {overall} overall)", cut as f64 / 16_000.0, rms(around));
        start = cut;
    }
    assert!(elapsed < Duration::from_secs(20), "the detector keeps up with the recording: {elapsed:?} for 3 minutes");
}

/// `bpe.vocab` (`piece<TAB>score` per line, what sherpa-onnx's `bpe_vocab` reads to encode hotwords)
/// from a sentencepiece `bpe.model`: the protobuf `ModelProto.pieces` (field 1) of `{ piece = 1,
/// score = 2 }`. The catalogue ships `bpe.model` only, so the hotword measurement derives it here.
fn bpe_vocab_from_model(model: &[u8]) -> String {
    fn varint(buf: &[u8], pos: &mut usize) -> u64 {
        let (mut out, mut shift) = (0u64, 0);
        loop {
            let b = buf[*pos];
            *pos += 1;
            out |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return out;
            }
            shift += 7;
        }
    }
    fn skip(buf: &[u8], pos: &mut usize, wire: u64) {
        match wire {
            0 => {
                varint(buf, pos);
            }
            1 => *pos += 8,
            2 => {
                let n = varint(buf, pos) as usize;
                *pos += n;
            }
            5 => *pos += 4,
            other => panic!("unexpected wire type {other}"),
        }
    }
    let mut out = String::new();
    let mut pos = 0;
    while pos < model.len() {
        let key = varint(model, &mut pos);
        if key >> 3 == 1 && key & 7 == 2 {
            let end = varint(model, &mut pos) as usize + pos;
            let (mut piece, mut score) = (String::new(), 0.0_f32);
            while pos < end {
                let k = varint(model, &mut pos);
                match (k >> 3, k & 7) {
                    (1, 2) => {
                        let n = varint(model, &mut pos) as usize;
                        piece = String::from_utf8(model[pos..pos + n].to_vec()).unwrap();
                        pos += n;
                    }
                    (2, 5) => {
                        score = f32::from_le_bytes(model[pos..pos + 4].try_into().unwrap());
                        pos += 4;
                    }
                    (_, wire) => skip(model, &mut pos, wire),
                }
            }
            out.push_str(&format!("{piece}\t{score}\n"));
        } else {
            skip(model, &mut pos, key & 7);
        }
    }
    out
}

/// One pass of the sample through a streaming recogniser the way `sherpa.rs` drives it (100 ms
/// chunks, decode while ready, commit + reset at endpoints, flush at the end). Returns the text and
/// the per-chunk decode times in microseconds.
fn stream_pass(recognizer: &sherpa_onnx::OnlineRecognizer, samples: &[f32]) -> (String, Vec<u128>) {
    let stream = recognizer.create_stream();
    let mut text = String::new();
    let mut times = Vec::new();
    for chunk in samples.chunks(LIVE_CHUNK_SAMPLES) {
        let step = Instant::now();
        stream.accept_waveform(16_000, chunk);
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }
        if recognizer.is_endpoint(&stream) {
            text.push_str(recognizer.get_result(&stream).map(|r| r.text).unwrap_or_default().trim());
            recognizer.reset(&stream);
        }
        times.push(step.elapsed().as_micros());
    }
    stream.input_finished();
    while recognizer.is_ready(&stream) {
        recognizer.decode(&stream);
    }
    text.push_str(recognizer.get_result(&stream).map(|r| r.text).unwrap_or_default().trim());
    (text, times)
}

/// Character edit distance (Levenshtein over `char`s).
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = if ca == cb { prev } else { 1 + prev.min(row[j]).min(row[j + 1]) };
            prev = cur;
        }
    }
    row[b.len()]
}

/// docs/dictation.md §16: are sherpa-onnx hotwords worth turning on for the catalogue's streaming
/// Zipformer? The sample (whose streaming transcript mis-hears 集成 as 提成) runs through five
/// recognisers built from the same five files — greedy search (what ships), modified beam search
/// without hotwords, and modified beam search with hotwords (`modeling_unit = bpe`, a `bpe.vocab`
/// derived from `bpe.model`): the sample's three words at 4 paths / score 1.5, at 10 paths / score
/// 3.0, and `集成` alone at 10 paths / score 3.0. Three passes each; load time, per-chunk decode time
/// (the live budget is 100 ms a chunk) and the text are printed and recorded in §16. Nothing is
/// asserted beyond "every configuration loads and recognises Chinese".
#[test]
#[ignore = "needs VOLTIP_LOCAL_STREAM_DIR (the downloaded streaming Zipformer directory) and VOLTIP_LOCAL_SAMPLE_WAV"]
fn real_streaming_hotwords_cost_and_effect() {
    let dir = PathBuf::from(std::env::var("VOLTIP_LOCAL_STREAM_DIR").expect("VOLTIP_LOCAL_STREAM_DIR"));
    let (samples, rate) = voltip_asr_local::decode_wav(&sample_wav()).unwrap();
    assert_eq!(rate, 16_000);
    let scratch = tempfile::tempdir().unwrap();
    let vocab = scratch.path().join("bpe.vocab");
    std::fs::write(&vocab, bpe_vocab_from_model(&std::fs::read(dir.join("bpe.model")).unwrap())).unwrap();
    let file = |name: &str| Some(dir.join(name).to_string_lossy().into_owned());
    let base = |method: &str| {
        let mut config = sherpa_onnx::OnlineRecognizerConfig {
            decoding_method: Some(method.to_owned()),
            max_active_paths: 4,
            enable_endpoint: true,
            rule1_min_trailing_silence: 2.0,
            rule2_min_trailing_silence: 0.8,
            rule3_min_utterance_length: 20.0,
            ..sherpa_onnx::OnlineRecognizerConfig::default()
        };
        config.model_config.transducer =
            sherpa_onnx::OnlineTransducerModelConfig { encoder: file("encoder.int8.onnx"), decoder: file("decoder.onnx"), joiner: file("joiner.int8.onnx") };
        config.model_config.tokens = file("tokens.txt");
        config.model_config.num_threads = 2;
        config.model_config.provider = Some("cpu".to_owned());
        config
    };
    // `modeling_unit = bpe`: every CJK character of this model is its own `▁X` piece, so a Chinese
    // hotword is written with spaces between its characters (`集 成` → `▁集 ▁成`).
    let hotwords = |words: &str, score: f32, paths: i32| {
        let mut config = base("modified_beam_search");
        config.max_active_paths = paths;
        config.model_config.modeling_unit = Some("bpe".to_owned());
        config.model_config.bpe_vocab = Some(vocab.to_string_lossy().into_owned());
        config.hotwords_buf = Some(words.replace('|', "\n").into_bytes());
        config.hotwords_score = score;
        config
    };
    let three = "集 成|teams|good idea";
    let configs = [
        ("greedy_search (ships)", base("greedy_search")),
        ("modified_beam_search 4 paths", base("modified_beam_search")),
        ("modified_beam_search 4 paths + 3 hotwords, score 1.5", hotwords(three, 1.5, 4)),
        ("modified_beam_search 10 paths + 3 hotwords, score 3.0", hotwords(three, 3.0, 10)),
        ("modified_beam_search 10 paths + 集成 only, score 3.0", hotwords("集 成", 3.0, 10)),
    ];
    let mut baseline = String::new();
    for (label, config) in configs {
        let load = Instant::now();
        let recognizer = sherpa_onnx::OnlineRecognizer::create(&config).unwrap_or_else(|| panic!("{label}: sherpa-onnx refused the configuration"));
        let load_ms = load.elapsed().as_millis();
        let mut all = Vec::new();
        let mut text = String::new();
        for _ in 0..3 {
            let (t, times) = stream_pass(&recognizer, &samples);
            all.extend(times);
            text = t;
        }
        let avg = all.iter().sum::<u128>() / all.len().max(1) as u128;
        let max = all.iter().copied().max().unwrap_or_default();
        let mut sorted = all.clone();
        sorted.sort_unstable();
        let p95 = sorted[sorted.len() * 95 / 100];
        if baseline.is_empty() {
            baseline = text.clone();
        }
        println!(
            "{label}: load={load_ms} ms chunks={} avg={avg} us p95={p95} us max={max} us edits_vs_greedy={} 集成={} 提成={} text={text}",
            all.len(),
            edit_distance(&baseline, &text),
            text.contains("集成"),
            text.contains("提成"),
        );
        assert!(is_chinese(&text), "{label}: {text}");
    }
}

/// docs/dictation.md §16: the default local tier cannot take a vocabulary prompt. transcribe.cpp
/// 0.2.3 builds the Qwen3-ASR chat prompt with an empty system turn and advertises no
/// `InitialPrompt` feature (whisper only); the Whisper run extension — the crate's one prompt
/// knob — is refused for this family.
#[test]
#[ignore = "needs VOLTIP_LOCAL_GGUF (a downloaded Qwen3-ASR Q6_K gguf) and VOLTIP_LOCAL_SAMPLE_WAV"]
fn real_qwen3_gguf_takes_no_vocabulary_prompt() {
    let gguf = PathBuf::from(std::env::var("VOLTIP_LOCAL_GGUF").expect("VOLTIP_LOCAL_GGUF"));
    let model = transcribe_cpp::Model::load(&gguf).unwrap();
    println!("arch={} variant={} initial_prompt={}", model.arch(), model.variant(), model.supports(transcribe_cpp::Feature::InitialPrompt));
    assert!(!model.supports(transcribe_cpp::Feature::InitialPrompt), "Qwen3-ASR in transcribe.cpp 0.2.3 takes no prompt");
    let (samples, _) = voltip_asr_local::decode_wav(&sample_wav()).unwrap();
    let mut session = model.session().unwrap();
    let options = transcribe_cpp::RunOptions {
        family: Some(transcribe_cpp::RunExtension::Whisper(transcribe_cpp::WhisperRunOptions {
            initial_prompt: Some("good idea, Teams".into()),
            ..transcribe_cpp::WhisperRunOptions::default()
        })),
        ..transcribe_cpp::RunOptions::default()
    };
    let refused = session.run(&samples, &options);
    println!("whisper prompt extension on qwen3: {:?}", refused.as_ref().map(|t| t.text.clone()));
    assert!(refused.is_err(), "the prompt extension is refused, not silently ignored");
}

/// docs/dictation.md §17: Qwen3-ASR 0.6B (transcribe.cpp, no language hint) answers the public
/// `zh.wav` sample (`VOLTIP_LOCAL_SAMPLE_WAV=/tmp/zh-hf.wav`, sha256 `b77f1794…e373`) in Traditional
/// characters; driven through the real dictation engine with the default settings, the text that
/// reaches the injector — and the history's raw text — is Simplified.
#[tokio::test]
#[ignore = "needs VOLTIP_LOCAL_GGUF (Qwen3-ASR GGUF) and VOLTIP_LOCAL_SAMPLE_WAV (the public zh.wav)"]
async fn real_qwen3_traditional_answer_reaches_the_pipeline_end_in_simplified() {
    use std::sync::Arc;
    use voltip_core::dictation::fakes::{FakeAudio, FakeInjector};
    use voltip_core::dictation::{DictationEngine, DictationPorts, Recording};
    use voltip_core::engines::UserSecrets;
    use voltip_core::{BuiltIn, ChineseScript, DictationPhase, EngineSettings, ResolvedEngines};

    let gguf = PathBuf::from(std::env::var("VOLTIP_LOCAL_GGUF").expect("VOLTIP_LOCAL_GGUF"));
    let id = if gguf.to_string_lossy().contains("1.7B") { "qwen3-asr-1.7b" } else { "qwen3-asr-0.6b" };
    let e = entry(id).unwrap();
    let root = tempfile::tempdir().unwrap();
    stage(root.path(), e, |_| gguf.clone());
    let wav = sample_wav();
    let (samples, rate) = voltip_asr_local::decode_wav(&wav).unwrap();
    let t = LocalTranscriber::new(root.path()).select(e.id);
    let raw = t.transcribe(&wav, None, &[]).await.unwrap().text;
    let converted = voltip_core::script::normalized(ChineseScript::Simplified, &raw);
    println!("model={id} raw={raw} simplified={converted}");
    assert!(is_chinese(&raw));

    let transcriber: Arc<dyn Transcriber> = Arc::new(t);
    let injector = Arc::new(FakeInjector::paste());
    let recording = Recording { wav: wav.clone(), duration_ms: samples.len() as u64 * 1000 / u64::from(rate), sample_rate_hz: rate };
    let ports = DictationPorts {
        audio: Arc::new(FakeAudio::recording(recording)),
        injector: injector.clone(),
        factory: Arc::new(move |_| (transcriber.clone(), None)),
        models: None,
        streaming: None,
        probe: None,
        service_probe: None,
        segmenter: None,
    };
    // The default settings: `chinese_script = simplified`.
    let settings = EngineSettings { refine_enabled: false, ..EngineSettings::default() };
    assert_eq!(settings.chinese_script, ChineseScript::Simplified);
    let engines = ResolvedEngines::resolve(&settings, &UserSecrets::default(), &BuiltIn::EMPTY);
    let (levels, _) = tokio::sync::broadcast::channel(8);
    let (mut engine, mut rx) = DictationEngine::new(ports, &engines, levels);
    engine.start().unwrap();
    for _ in 0..2 {
        let ev = tokio::time::timeout(Duration::from_secs(30), rx.recv()).await.unwrap().unwrap();
        engine.on_internal(ev);
    }
    engine.stop().unwrap();
    let started = Instant::now();
    while !engine.status().phase.is_terminal() {
        let ev = tokio::time::timeout(Duration::from_secs(120), rx.recv()).await.unwrap().unwrap();
        engine.on_internal(ev);
    }
    let DictationPhase::Done { text, raw_text, .. } = engine.status().phase.clone() else { panic!("{:?}", engine.status().phase) };
    println!("pipeline ({} ms): raw_text={raw_text} text={text}", started.elapsed().as_millis());
    assert_eq!(text, converted, "the injected text is the Simplified form of the recognition");
    assert_eq!(voltip_core::script::normalized(ChineseScript::Simplified, &text), text, "nothing Traditional is left");
    assert_eq!(raw_text, text, "the history's raw text is normalised too");
    assert_eq!(injector.injected(), vec![text.clone()]);
    if raw != converted {
        println!("the recogniser answered in Traditional; the pipeline turned it into Simplified");
    }
    assert!(text.contains("开放时间"), "the sample says 开放时间: {text}");
}

/// docs/dictation.md §10.6: the compute choice reaches transcribe.cpp. `cpu` runs on the CPU; on a
/// build with a GPU backend `gpu` runs on the named GPU and `auto` picks a GPU too; both give the
/// same kind of text. Prints the machine, the backends and the latencies.
#[tokio::test]
#[ignore = "needs VOLTIP_LOCAL_GGUF and VOLTIP_LOCAL_SAMPLE_WAV; the GPU half needs a build with a GPU backend"]
async fn real_gguf_runs_where_the_compute_choice_says() {
    let gguf = PathBuf::from(std::env::var("VOLTIP_LOCAL_GGUF").expect("VOLTIP_LOCAL_GGUF"));
    let id = if gguf.to_string_lossy().contains("1.7B") { "qwen3-asr-1.7b" } else { "qwen3-asr-0.6b" };
    let e = entry(id).unwrap();
    let root = tempfile::tempdir().unwrap();
    stage(root.path(), e, |_| gguf.clone());
    let wav = sample_wav();
    let machine = hardware();
    eprintln!("hardware: {} CPU threads, GPUs {:?}", machine.cpu_threads, machine.gpus);
    let run = |compute: Compute| {
        let t = LocalTranscriber::new(root.path()).select(e.id).with_compute(compute);
        let wav = wav.clone();
        async move {
            let started = Instant::now();
            let out = t.transcribe(&wav, None, &[]).await.unwrap();
            let cold = started.elapsed();
            let started = Instant::now();
            t.transcribe(&wav, None, &[]).await.unwrap();
            let warm = started.elapsed();
            (t.loaded_backend().unwrap_or_default(), out.text, cold, warm)
        }
    };
    let (backend, text, cold, warm) = run(Compute { device: LocalDevice::Cpu, ..Compute::default() }).await;
    eprintln!("cpu: backend {backend}, cold {cold:?}, warm {warm:?}: {text}");
    assert!(backend.to_ascii_lowercase().contains("cpu"), "{backend}");
    assert!(is_chinese(&text), "{text}");
    let Some(gpu) = machine.gpus.first() else {
        eprintln!("no GPU backend in this build: the GPU half is skipped");
        return;
    };
    let (backend, text, cold, warm) = run(Compute { device: LocalDevice::Gpu, gpu: Some(gpu.name.clone()), threads: None }).await;
    eprintln!("gpu {}: backend {backend}, cold {cold:?}, warm {warm:?}: {text}", gpu.name);
    assert!(!backend.to_ascii_lowercase().contains("cpu"), "asked for {}, ran on {backend}", gpu.name);
    assert!(is_chinese(&text), "{text}");
    let (backend, _, _, _) = run(Compute::default()).await;
    eprintln!("auto: backend {backend}");
    assert!(!backend.to_ascii_lowercase().contains("cpu"), "auto on a GPU build ran on {backend}");
}

/// Times every decode of the transcriber it wraps: audio length in, time spent (M9).
struct Timed {
    inner: std::sync::Arc<LocalTranscriber>,
    calls: std::sync::Mutex<Vec<(u64, u64)>>,
}

#[async_trait::async_trait]
impl Transcriber for Timed {
    async fn transcribe(
        &self,
        wav: &[u8],
        language: Option<&str>,
        glossary: &[String],
    ) -> Result<voltip_core::dictation::Transcript, voltip_core::dictation::DictationError> {
        let audio_ms = voltip_core::dictation::wav::pcm_data(wav).map_or(0, |pcm| pcm.len() as u64 / 32);
        let started = Instant::now();
        let out = self.inner.transcribe(wav, language, glossary).await;
        self.calls.lock().unwrap().push((audio_ms, started.elapsed().as_millis() as u64));
        out
    }
}

/// M9, an experiment (docs/dictation.md §11.8; user request 2026-09-30, plan item M9): the live
/// preview from the local Qwen3-ASR model, previewing the way the built-in service does —
/// `RedecodeStreaming` over `LocalTranscriber`, the sample fed at the microphone's pace. Prints
/// every decode (audio length and time), p50 / p95, the preview timeline, and the share of the
/// wall time spent decoding. The bar for offering it (the plan): p95 < 1.2 s with the 0.6B model
/// on the Windows build host. Data only: nothing in the app uses it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs VOLTIP_LOCAL_GGUF (a downloaded Qwen3-ASR Q6_K gguf) and VOLTIP_LOCAL_SAMPLE_WAV; an experiment, not a gate"]
async fn real_qwen3_redecode_preview_timing() {
    let gguf = PathBuf::from(std::env::var("VOLTIP_LOCAL_GGUF").expect("VOLTIP_LOCAL_GGUF"));
    let id = if gguf.to_string_lossy().contains("1.7B") { "qwen3-asr-1.7b" } else { "qwen3-asr-0.6b" };
    let e = entry(id).unwrap();
    let root = tempfile::tempdir().unwrap();
    stage(root.path(), e, |_| gguf.clone());
    let wav = sample_wav();
    let local = std::sync::Arc::new(LocalTranscriber::new(root.path()).select(e.id));
    // Loaded before the take, as the warm-up does (docs/dictation.md §10.7).
    let load = Instant::now();
    local.transcribe(&wav, None, &[]).await.unwrap();
    println!("model={id} load + first decode of the whole sample: {} ms", load.elapsed().as_millis());
    // The default sentence cap (20 s, the built-in service's), then 8 s to bound every decode.
    for longest in [Duration::from_secs(20), Duration::from_secs(8)] {
        let timed = std::sync::Arc::new(Timed { inner: local.clone(), calls: std::sync::Mutex::new(Vec::new()) });
        let params = voltip_core::dictation::redecode::RedecodeParams { longest, ..Default::default() };
        let streaming = voltip_core::dictation::redecode::RedecodeStreaming::with_params(timed.clone(), Vec::new(), params);
        let (samples, rate) = voltip_asr_local::decode_wav(&wav).unwrap();
        assert_eq!(rate, 16_000, "the sample must be 16 kHz mono");
        let (events, fin, wall_ms) = tokio::task::spawn_blocking(move || {
            let mut session = streaming.open(Some("zh")).unwrap();
            let started = Instant::now();
            let mut events = Vec::new();
            for (i, chunk) in samples.chunks(LIVE_CHUNK_SAMPLES).enumerate() {
                if let Some(wait) = Duration::from_millis(100 * i as u64).checked_sub(started.elapsed()) {
                    std::thread::sleep(wait);
                }
                session.feed(chunk);
                loop {
                    match session.poll() {
                        StreamEvent::Idle => break,
                        event => events.push((started.elapsed().as_millis(), event)),
                    }
                }
            }
            let fin = session.finish().unwrap();
            (events, fin, started.elapsed().as_millis())
        })
        .await
        .unwrap();
        println!("--- longest={longest:?}");
        for (at, event) in &events {
            println!("{at:>6} ms  {event:?}");
        }
        let calls = timed.calls.lock().unwrap().clone();
        let mut times: Vec<u64> = calls.iter().map(|&(_, t)| t).collect();
        times.sort_unstable();
        let pick = |q: f64| times[((times.len() as f64 - 1.0) * q).round() as usize];
        let busy: u64 = times.iter().sum();
        println!("decodes (audio ms, decode ms): {calls:?}");
        println!(
            "p50={} ms p95={} ms max={} ms; decoding {busy} ms of {wall_ms} ms wall ({}%); final={fin:?}",
            pick(0.5),
            pick(0.95),
            times.last().unwrap(),
            busy * 100 / u64::try_from(wall_ms.max(1)).unwrap()
        );
        assert!(events.iter().any(|(_, e)| matches!(e, StreamEvent::Partial { .. })), "a preview came back while the sample played");
        let text: String = fin.committed.iter().map(|s| s.text.as_str()).chain(std::iter::once(fin.tail.as_str())).collect();
        assert!(text.contains("想创建"), "the sample says 想创建: {text}");
    }
}
