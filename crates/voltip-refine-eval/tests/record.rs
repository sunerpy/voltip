//! The repository's samples and the last evaluation's record (docs/refine-eval.md): every preset and
//! scene has samples that exercise every part of the prompt, and the prompts the app sends are the
//! ones the last evaluation measured. These tests run with the others in CI; the evaluation itself
//! (Bedrock, the judge, the report) runs only by hand (`make refine-eval`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use voltip_refine_eval::{PRIMARY_MODEL, RECORD_PATH, Record, Requests, SAMPLES_PATH, SampleSet, Unit, digests, stale_message};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn samples() -> SampleSet {
    SampleSet::load(&root().join(SAMPLES_PATH)).unwrap_or_else(|e| panic!("{e}"))
}

/// Each preset and each scene with an instruction is measured on enough takes to tell a change.
#[test]
fn every_unit_has_its_samples() {
    let set = samples();
    for unit in Unit::all() {
        let count = set.samples.iter().filter(|s| s.unit == unit).count();
        assert!(count >= 5, "{unit}: {count} samples, at least 5 are needed");
    }
}

/// The samples go through every block of the system prompt: a change to any of them changes a
/// digest. The language line with and without 「输出保持这种语言」, the application and window-title
/// lines, the scene instruction and the glossary.
#[test]
fn the_samples_exercise_every_part_of_the_prompt() {
    let set = samples();
    let requests = Requests::new();
    let systems: Vec<String> =
        set.samples.iter().map(|s| requests.body(s, PRIMARY_MODEL).unwrap()["messages"][0]["content"].as_str().unwrap().to_owned()).collect();
    let parts = [
        ("the language line", "\n\n说话人使用的语言代码：zh。输出保持这种语言。"),
        ("the language line of a translation", "\n\n说话人使用的语言代码：zh。\n"),
        ("the English language line", "说话人使用的语言代码：en。"),
        ("the context clause", voltip_refine::CONTEXT_CLAUSE),
        ("the application line", "\n当前应用："),
        ("the window-title line", "\n窗口标题："),
        ("the instruction clause", voltip_refine::INSTRUCTION_CLAUSE),
        ("the glossary clause", voltip_refine::GLOSSARY_CLAUSE),
        ("the output contract", voltip_refine::OUTPUT_CONTRACT),
    ];
    for (what, text) in parts {
        let found = systems.iter().any(|s| s.contains(text) || (text.ends_with('\n') && s.ends_with(text.trim_end_matches('\n'))));
        assert!(found, "no sample's prompt carries {what}: {text:?}");
    }
    let without_glossary = set.samples.iter().zip(&systems).filter(|(s, p)| matches!(s.unit, Unit::Preset(_)) && !p.contains(voltip_refine::GLOSSARY_CLAUSE));
    assert!(without_glossary.count() > 0, "presets are measured without a dictionary too");
}

/// The CI check: the prompts the app sends are the ones the report measured. A change to a preset,
/// a scene instruction, a shared clause, a term pack, the request's parameters or a sample fails
/// here until the evaluation runs again and its record is committed.
#[test]
fn the_last_evaluation_measured_the_prompts_the_app_sends() {
    let set = samples();
    let record = Record::load(&root().join(RECORD_PATH)).unwrap_or_else(|e| panic!("{e}: run {} first", voltip_refine_eval::RERUN_COMMAND));
    let current = digests(&set.samples).unwrap();
    let stale = record.stale(&current);
    assert!(stale.is_empty(), "{}", stale_message(&stale, &record));
    assert!(root().join(&record.report).is_file(), "the record names a report that is not there: {}", record.report);
}
