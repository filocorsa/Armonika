//! Deterministic fake music libraries for benchmarks and tests.

mod big;
pub mod names;

pub use big::{
    AudioFormat, BigLibrarySpec, GeneratedTrack, generate_big_library, plan_big_library,
};
