use smolgen::{Build, Emitter, Gitcl, Rustc};
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut emitter = Emitter::default();
    emitter
        .default_on_error()
        .add_instructions(&Build::builder().build_timestamp(true).build())?
        .add_instructions(&Rustc::builder().semver(true).build())?
        .add_instructions(&Gitcl::builder().sha(true).dirty(true).build())?;

    let mut instructions = Vec::new();
    emitter.emit_to(&mut instructions)?;
    // Tracking the git HEAD reruns this script, and rebuilds the crate, on every commit and
    // checkout. Debug builds skip it and keep the SHA from the last run of this script.
    // Release profiles keep tracking HEAD, so their version always matches the checkout.
    let skip_head_tracking = env::var("PROFILE")? == "debug";
    for line in String::from_utf8(instructions)?.lines() {
        let is_git_path = line.starts_with("cargo:rerun-if-changed=")
            && line != "cargo:rerun-if-changed=build.rs";
        if !(skip_head_tracking && is_git_path) {
            println!("{line}");
        }
    }
    Ok(())
}
