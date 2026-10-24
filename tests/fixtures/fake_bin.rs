//! Stand-in for a Foundry binary, compiled by the integration tests.
//!
//! Prints its name and the name of its directory, which is the toolchain it belongs to. Unless
//! asked for its version with `-V`, it also prints its arguments and `FOUNDRYUP_TOOLCHAIN`, and
//! `exit <code>` exits with `code`.

fn main() {
    let exe = std::env::current_exe().unwrap();
    let name = exe.file_stem().unwrap().to_string_lossy();
    let toolchain = exe.parent().unwrap().file_name().unwrap().to_string_lossy();
    println!("{name} {toolchain}");

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args != ["-V"] {
        println!("args: {args:?}");
        println!("FOUNDRYUP_TOOLCHAIN: {:?}", std::env::var("FOUNDRYUP_TOOLCHAIN").ok());
    }
    if args.first().is_some_and(|arg| arg == "exit") {
        std::process::exit(args[1].parse().unwrap());
    }
}
