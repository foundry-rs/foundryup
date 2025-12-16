fn main() {
    if let Err(e) = vergen_git2::Emitter::default()
        .add_instructions(
            &vergen_git2::BuildBuilder::default().build_timestamp(true).build().unwrap(),
        )
        .unwrap()
        .add_instructions(&vergen_git2::RustcBuilder::default().semver(true).build().unwrap())
        .unwrap()
        .add_instructions(
            &vergen_git2::Git2Builder::default().sha(true).dirty(true).build().unwrap(),
        )
        .unwrap()
        .emit()
    {
        println!("cargo:warning=vergen failed: {e}");
    }
}
