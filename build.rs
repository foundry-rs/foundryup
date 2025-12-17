use vergen_gitcl as vergen;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    vergen::Emitter::default()
        .add_instructions(&vergen::BuildBuilder::default().build_timestamp(true).build()?)?
        .add_instructions(&vergen::RustcBuilder::default().semver(true).build()?)?
        .add_instructions(&vergen::GitclBuilder::default().sha(true).dirty(true).build()?)?
        .emit()?;
    Ok(())
}
