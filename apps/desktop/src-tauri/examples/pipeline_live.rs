//! Manual, network-dependent check of the real ASR + refine clients against the built-in defaults
//! baked in at compile time (`.env.build`). Not a test: run it by hand with
//! `set -a; . .env.build; set +a; cargo run -p voltip-desktop --example pipeline_live -- <file.wav>`.
//! Prints the transcript, the refined text and the latencies; exits non-zero on any failure.

use std::time::Duration;

use voltip_core::engines::{BuiltIn, EngineSettings, ResolvedEngines, UserSecrets};
use voltip_desktop_lib::dictation::build_clients;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: pipeline_live <file.wav>")?;
    let wav = std::fs::read(&path)?;
    let built_in = BuiltIn::from_build();
    let resolved = ResolvedEngines::resolve(&EngineSettings::default(), &UserSecrets::default(), &built_in);
    println!("asr model: {}  refine model: {}  (hosts come from the build; see docs/runbook.md)", resolved.asr_model, resolved.refine_model);
    // The example only exercises the cloud path; the local transcriber needs no model here.
    let local = voltip_asr_local::LocalTranscriber::new(std::env::temp_dir().join("voltip-example-models"));
    let (asr, refiner) = build_clients(&resolved, &local);
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(async {
        let t = tokio::time::timeout(Duration::from_secs(90), asr.transcribe(&wav, Some("zh"), &[])).await??;
        println!("transcript ({} ms): {}", t.latency_ms, t.text);
        if let Some(r) = refiner {
            let out = tokio::time::timeout(Duration::from_secs(60), r.refine(&t.text, &voltip_core::dictation::RefineHints::default())).await??;
            println!("refined ({} ms): {}", out.latency_ms, out.text);
        } else {
            println!("refine: disabled / unconfigured");
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}
