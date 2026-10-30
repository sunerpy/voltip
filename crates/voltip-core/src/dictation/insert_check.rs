//! Whether a paste reached the focused field (docs/dictation.md §28). The rules are in
//! `voltip_platform::insert_check::rules`, next to the applications the check skips, so the
//! desktop's platform readers (`voltip-focus`) decide exactly as the core does without building
//! it; this module is their place in the core.

pub use voltip_platform::insert_check::rules::*;
