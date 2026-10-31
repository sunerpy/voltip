#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Recognition quality and speed by input length (docs/dictation.md §29): the measurement behind
//! each model's `max_piece` and the transcriber's `LOCK_WAIT`. Every clip of the corpus
//! (`scripts/measure/make-corpus.py`) is recognised in one call, without the transcriber's piece
//! cuts, by the production loader on the CPU with the engine's default threads.
//!
//! ```text
//! cargo run --release -p voltip-asr-local --example piece_quality -- \
//!   --model sense-voice-small --dir <model directory> --manifest /tmp/voltip-measure/corpus/manifest.csv \
//!   [--lengths 30,60,90,119,121] [--machine linux] [--hyp-dir <directory>] [--header]
//! ```
//!
//! `--dir` holds the catalogue entry's files (`model.int8.onnx` + `tokens.txt`, or the GGUF). The
//! manifest's paths are used as written, or else found beside the manifest (`<lang>/<file>`), so a
//! corpus copied to another machine works. Output, on stdout, CSV rows behind a tag (`scripts/measure/
//! local-pieces.sh` collects them):
//!
//! * `piece,machine,model,backend,lang,name,length_s,speech_s,load_ms,infer_ms,rtf,metric,edits,ref_units,rate,status`
//!   per clip: `metric` is `cer` (characters, Chinese in simplified characters) or `wer` (words,
//!   English), both without punctuation; `status` is `ok` or `error:<kind>`.
//! * `peak,machine,model,length_s,peak_rss_mb` after each length: the process's peak memory so far
//!   (Linux `VmHWM`, the number `/usr/bin/time -v` reports; Windows `PeakWorkingSet64`).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use voltip_asr_local::transcriber::language_for;
use voltip_asr_local::{Abort, Compute, DefaultLoader, LocalDevice, Recognizer, RecognizerLoader, decode_wav, entry};
use voltip_core::ChineseScript;

/// One clip of the manifest.
struct Clip {
    lang: String,
    name: String,
    length_s: u32,
    speech_s: String,
    wav: PathBuf,
    reference: PathBuf,
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// `path` as written, or else `<manifest dir>/<lang>/<file name>`.
fn locate(path: &str, manifest_dir: &Path, lang: &str) -> PathBuf {
    let written = PathBuf::from(path);
    if written.exists() {
        return written;
    }
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    manifest_dir.join(lang).join(file)
}

fn read_manifest(manifest: &Path) -> Vec<Clip> {
    let dir = manifest.parent().unwrap_or(Path::new("."));
    let text = std::fs::read_to_string(manifest).expect("manifest");
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().expect("manifest header").split(',').collect();
    let column = |name: &str| header.iter().position(|h| *h == name).unwrap_or_else(|| panic!("manifest has no {name} column"));
    let (lang, name, length, speech, wav, reference) = (column("lang"), column("name"), column("length_s"), column("speech_s"), column("wav"), column("ref"));
    lines
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let cells: Vec<&str> = line.split(',').collect();
            Clip {
                lang: cells[lang].to_owned(),
                name: cells[name].to_owned(),
                length_s: cells[length].parse().expect("length_s"),
                speech_s: cells[speech].to_owned(),
                wav: locate(cells[wav], dir, cells[lang]),
                reference: locate(cells[reference], dir, cells[lang]),
            }
        })
        .collect()
}

/// The language hint of a corpus language (FLEURS codes).
fn hint(lang: &str) -> &'static str {
    if lang.starts_with("cmn") || lang.starts_with("zh") { "zh" } else { "en" }
}

/// The text without SenseVoice-style `<|…|>` tags.
fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<|") {
        out.push_str(&rest[..start]);
        match rest[start..].find("|>") {
            Some(end) => rest = &rest[start + end + 2..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Chinese: the characters, letters and digits of the simplified text, lower case.
fn characters(text: &str) -> Vec<char> {
    voltip_core::script::normalized(ChineseScript::Simplified, &strip_tags(text)).chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// English: the words, lower case, without punctuation (an apostrophe inside a word stays).
fn words(text: &str) -> Vec<String> {
    strip_tags(text)
        .to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .map(|w| w.trim_matches('\''))
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Levenshtein distance.
fn edits<T: PartialEq>(reference: &[T], hypothesis: &[T]) -> usize {
    let mut previous: Vec<usize> = (0..=hypothesis.len()).collect();
    let mut current = vec![0; hypothesis.len() + 1];
    for (i, r) in reference.iter().enumerate() {
        current[0] = i + 1;
        for (j, h) in hypothesis.iter().enumerate() {
            current[j + 1] = (previous[j] + usize::from(r != h)).min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[hypothesis.len()]
}

/// The metric, the edits and the reference's length for one clip.
fn score(lang: &str, reference: &str, hypothesis: &str) -> (&'static str, usize, usize) {
    if hint(lang) == "zh" {
        let (r, h) = (characters(reference), characters(hypothesis));
        ("cer", edits(&r, &h), r.len())
    } else {
        let (r, h) = (words(reference), words(hypothesis));
        ("wer", edits(&r, &h), r.len())
    }
}

#[cfg(target_os = "linux")]
fn peak_rss_mb() -> Option<f64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb: f64 = status.lines().find_map(|l| l.strip_prefix("VmHWM:"))?.trim().trim_end_matches("kB").trim().parse().ok()?;
    Some(kb / 1024.0)
}

#[cfg(windows)]
fn peak_rss_mb() -> Option<f64> {
    let query = format!("(Get-Process -Id {}).PeakWorkingSet64", std::process::id());
    let out = std::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &query]).output().ok()?;
    let bytes: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    Some(bytes / 1_048_576.0)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn peak_rss_mb() -> Option<f64> {
    None
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let model = arg(&args, "--model").expect("--model <catalogue id>");
    let dir = PathBuf::from(arg(&args, "--dir").expect("--dir <model directory>"));
    let manifest = PathBuf::from(arg(&args, "--manifest").expect("--manifest <manifest.csv>"));
    let machine = arg(&args, "--machine").unwrap_or_else(|| std::env::consts::OS.to_owned());
    let lengths: Vec<u32> =
        arg(&args, "--lengths").unwrap_or_else(|| "30,60,90,119,121".into()).split(',').map(|l| l.trim().parse().expect("--lengths")).collect();
    let hyp_dir = arg(&args, "--hyp-dir").map(PathBuf::from);
    let entry = entry(&model).unwrap_or_else(|| panic!("{model} is not in the catalogue"));
    if let Some(hyp_dir) = &hyp_dir {
        std::fs::create_dir_all(hyp_dir).expect("--hyp-dir");
    }
    if args.iter().any(|a| a == "--header") {
        println!("piece,machine,model,backend,lang,name,length_s,speech_s,load_ms,infer_ms,rtf,metric,edits,ref_units,rate,status");
        println!("peak,machine,model,length_s,peak_rss_mb");
    }
    let clips = read_manifest(&manifest);
    let languages: BTreeSet<&str> = clips.iter().map(|c| c.lang.as_str()).filter(|l| entry.languages.contains(&hint(l))).collect();
    let compute = Compute { device: LocalDevice::Cpu, ..Compute::default() };
    // The recogniser in memory and the language it was loaded for.
    let mut loaded: Option<(Option<&'static str>, Box<dyn Recognizer>, u128)> = None;
    for &length in &lengths {
        for &lang in &languages {
            let language = language_for(entry.engine, Some(hint(lang)));
            if loaded.as_ref().is_none_or(|(l, _, _)| *l != language) {
                // The one before goes first: one recogniser in memory, as in the app.
                drop(loaded.take());
                let started = Instant::now();
                let recognizer = DefaultLoader.load(entry, &dir, language, &compute).unwrap_or_else(|e| panic!("{model}: {e}"));
                loaded = Some((language, recognizer, started.elapsed().as_millis()));
            }
            let (_, recognizer, load_ms) = loaded.as_mut().unwrap();
            for clip in clips.iter().filter(|c| c.lang == lang && c.length_s == length) {
                let (samples, rate) = decode_wav(&std::fs::read(&clip.wav).unwrap_or_else(|e| panic!("{}: {e}", clip.wav.display()))).expect("wav");
                let reference = std::fs::read_to_string(&clip.reference).expect("reference");
                let started = Instant::now();
                let outcome = recognizer.transcribe(rate, &samples, &Abort::new());
                let infer_ms = started.elapsed().as_millis();
                let (text, status) = match outcome {
                    Ok(text) => (text, "ok".to_owned()),
                    Err(e) => {
                        eprintln!("{model} {}: {} ({})", clip.name, e.message, e.kind);
                        (String::new(), format!("error:{}", e.kind))
                    }
                };
                let (metric, edits, units) = score(lang, &reference, &text);
                let rate_pct = if units == 0 { 0.0 } else { edits as f64 * 100.0 / units as f64 };
                let rtf = infer_ms as f64 / (f64::from(length) * 1000.0);
                println!(
                    "piece,{machine},{model},{},{lang},{},{length},{},{load_ms},{infer_ms},{rtf:.4},{metric},{edits},{units},{rate_pct:.2},{status}",
                    recognizer.backend(),
                    clip.name,
                    clip.speech_s
                );
                if let Some(hyp_dir) = &hyp_dir {
                    std::fs::write(hyp_dir.join(format!("{machine}-{model}-{}.txt", clip.name)), text.trim()).expect("hypothesis");
                }
                // The load counts once.
                *load_ms = 0;
            }
        }
        let peak = peak_rss_mb().map_or_else(String::new, |mb| format!("{mb:.1}"));
        println!("peak,{machine},{model},{length},{peak}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores_ignore_punctuation_case_tags_and_the_script() {
        assert_eq!(edits(&['a', 'b', 'c'], &['a', 'x', 'c', 'd']), 2);
        assert_eq!(edits::<char>(&[], &['a']), 1);
        assert_eq!(score("cmn_hans_cn", "开放时间：早上九点。", "<|zh|><|NEUTRAL|>開放時間早上九點"), ("cer", 0, 8));
        assert_eq!(score("en_us", "It's easier, for lava!", "it's easier for the lava"), ("wer", 1, 4));
        assert_eq!(strip_tags("<|en|>hi <|x"), "hi ");
        assert_eq!(hint("en_us"), "en");
    }
}
