//! `voltip-refine-eval`: the requests and digests of the AI polish prompt evaluation, for
//! `scripts/refine-eval/refine_eval.py` (docs/refine-eval.md). Sends nothing.
//!
//! ```text
//! voltip-refine-eval requests [--samples PATH] [--model NAME]   one JSON line per sample: id, unit, body
//! voltip-refine-eval clean                                      JSON array of answers in, cleaned out
//! voltip-refine-eval digests [--samples PATH]                   the units' digests as a JSON object
//! voltip-refine-eval lengths                                    each unit's prompt length and limit
//! ```
//!
//! `--samples` defaults to the repository's samples (run from its root), `--model` to the built-in
//! primary model as the app names it.

#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use voltip_refine_eval::{PRIMARY_MODEL, Requests, SAMPLES_PATH, SampleSet, digests, lengths};

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            let _ = writeln!(std::io::stderr(), "voltip-refine-eval: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let (command, rest) = args.split_first().ok_or("usage: voltip-refine-eval requests|clean|digests|lengths [--samples PATH] [--model NAME]")?;
    let mut samples = PathBuf::from(SAMPLES_PATH);
    let mut model = PRIMARY_MODEL.to_owned();
    let mut options = rest.iter();
    while let Some(option) = options.next() {
        let value = options.next().ok_or_else(|| format!("{option} needs a value"))?;
        match option.as_str() {
            "--samples" => samples = PathBuf::from(value),
            "--model" => model.clone_from(value),
            other => return Err(format!("unknown option {other}")),
        }
    }
    let mut out = std::io::stdout().lock();
    match command.as_str() {
        "requests" => {
            let set = SampleSet::load(&samples).map_err(|e| e.to_string())?;
            let requests = Requests::new();
            for sample in &set.samples {
                let body = requests.body(sample, &model).map_err(|e| e.to_string())?;
                let line = serde_json::json!({ "id": sample.id, "unit": sample.unit.name(), "body": body });
                writeln!(out, "{line}").map_err(|e| e.to_string())?;
            }
        }
        "clean" => {
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input).map_err(|e| e.to_string())?;
            let answers: Vec<String> = serde_json::from_str(&input).map_err(|e| format!("clean wants a JSON array of strings: {e}"))?;
            let cleaned: Vec<String> = answers.iter().map(|a| voltip_refine::clean_answer(a)).collect();
            writeln!(out, "{}", serde_json::Value::from(cleaned)).map_err(|e| e.to_string())?;
        }
        "digests" => {
            let set = SampleSet::load(&samples).map_err(|e| e.to_string())?;
            let digests = digests(&set.samples).map_err(|e| e.to_string())?;
            writeln!(out, "{}", serde_json::to_string_pretty(&digests).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        }
        "lengths" => {
            let rows: Vec<serde_json::Value> =
                lengths().into_iter().map(|l| serde_json::json!({ "unit": l.unit.name(), "chars": l.chars, "limit": l.limit })).collect();
            writeln!(out, "{}", serde_json::Value::from(rows)).map_err(|e| e.to_string())?;
        }
        other => return Err(format!("unknown command {other}")),
    }
    Ok(())
}
