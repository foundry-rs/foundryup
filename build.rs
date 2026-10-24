use smolgen::{Build, Emitter, Gitcl, Rustc};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Emitter::default()
        .default_on_error()
        .add_instructions(&Build::builder().build_timestamp(true).build())?
        .add_instructions(&Rustc::builder().semver(true).build())?
        .add_instructions(&Gitcl::builder().sha(true).dirty(true).build())?
        .emit()?;
    Ok(())
}
