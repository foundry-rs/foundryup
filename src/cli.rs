use clap::{CommandFactory, Parser, Subcommand};

/// The installer for Foundry.
///
/// Update or revert to a specific Foundry version with ease.
///
/// By default, the latest stable version is installed from built binaries.
#[derive(Debug, Parser)]
#[command(name = "foundryup", version = crate::config::LONG_VERSION, about)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[command(flatten)]
    pub install_args: InstallArgs,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Commands {
    /// Install a specific version (default command when no subcommand given)
    #[command(visible_alias = "i")]
    Install(InstallArgs),

    /// List installed versions
    #[command(visible_alias = "ls")]
    List,

    /// Use a specific installed version
    #[command(name = "use")]
    Use {
        /// Version to use (e.g., stable, nightly, v0.8.0)
        version: String,
    },

    /// Update foundryup to the latest version
    Update,

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },
}

#[derive(Debug, Clone, Default, Parser)]
pub(crate) struct InstallArgs {
    /// Install a specific version from built binaries (e.g., stable, nightly, 0.3.0)
    #[arg(id = "ver", short = 'i', long = "install", value_name = "VERSION")]
    pub version: Option<String>,

    /// Build and install a specific branch
    #[arg(short = 'b', long)]
    pub branch: Option<String>,

    /// Build and install a specific Pull Request
    #[arg(short = 'P', long)]
    pub pr: Option<u64>,

    /// Build and install a specific commit
    #[arg(short = 'C', long)]
    pub commit: Option<String>,

    /// Build and install from a remote GitHub repo (uses default branch if no other options)
    #[arg(short = 'r', long)]
    pub repo: Option<String>,

    /// Build and install a local repository
    #[arg(short = 'p', long)]
    pub path: Option<std::path::PathBuf>,

    /// Number of CPUs to use for building (default: all)
    #[arg(short = 'j', long)]
    pub jobs: Option<u32>,

    /// Skip SHA verification (INSECURE)
    #[arg(short = 'f', long)]
    pub force: bool,

    /// Install binaries for a specific network (e.g., tempo)
    #[arg(short = 'n', long)]
    pub network: Option<Network>,

    /// Install a specific architecture (amd64, arm64)
    #[arg(long)]
    pub arch: Option<String>,

    /// Install a specific platform (win32, linux, darwin, alpine)
    #[arg(long)]
    pub platform: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Network {
    Tempo,
}

pub(crate) fn print_completions(shell: clap_complete::Shell) {
    clap_complete::generate(shell, &mut Cli::command(), "foundryup", &mut std::io::stdout());
}
