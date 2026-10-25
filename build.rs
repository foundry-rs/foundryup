use smolgen::{Build, Emitter, Gitcl, Rustc};
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Debug builds skip git HEAD tracking, which would otherwise rerun this script, and rebuild
    // the crate, on every commit and checkout. Release profiles keep tracking HEAD.
    let track_head = env::var("PROFILE")? != "debug";
    Emitter::default()
        .default_on_error()
        .add_instructions(&Build::builder().build_timestamp(true).build())?
        .add_instructions(&Rustc::builder().semver(true).build())?
        .add_instructions(
            &Gitcl::builder().sha(true).dirty(true).rerun_on_head_change(track_head).build(),
        )?
        .emit()?;
    Ok(())
}
