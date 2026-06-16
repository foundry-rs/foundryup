use eyre::{Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    Linux,
    Alpine,
    Darwin,
    Win32,
}

impl Platform {
    pub(crate) fn detect() -> Result<Self> {
        // Linux defaults to the glibc `linux` artifact; the `alpine` (musl)
        // build is opt-in via `--platform`.
        if cfg!(target_os = "linux") {
            Ok(Self::Linux)
        } else if cfg!(target_os = "macos") {
            Ok(Self::Darwin)
        } else if cfg!(target_os = "windows") {
            Ok(Self::Win32)
        } else {
            bail!("unsupported platform: {}", std::env::consts::OS)
        }
    }

    pub(crate) fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "linux" => Ok(Self::Linux),
            "alpine" => Ok(Self::Alpine),
            "darwin" => Ok(Self::Darwin),
            s if s.starts_with("mac") => Ok(Self::Darwin),
            s if s.starts_with("mingw") || s.starts_with("win") => Ok(Self::Win32),
            _ => bail!("unsupported platform: {s}"),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Alpine => "alpine",
            Self::Darwin => "darwin",
            Self::Win32 => "win32",
        }
    }

    pub(crate) fn archive_ext(self) -> &'static str {
        match self {
            Self::Win32 => "zip",
            _ => "tar.gz",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Arch {
    Amd64,
    Arm64,
}

impl Arch {
    pub(crate) fn detect() -> Self {
        Self::normalize(std::env::consts::ARCH)
    }

    pub(crate) fn from_str(s: &str) -> Self {
        Self::normalize(s)
    }

    /// Normalizes an arch name, defaulting to `amd64` for anything unrecognized.
    /// A literal `x86_64` resolves to `arm64` when running under Rosetta.
    fn normalize(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "x86_64" if is_rosetta() => Self::Arm64,
            "arm64" | "aarch64" => Self::Arm64,
            _ => Self::Amd64,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Target {
    pub platform: Platform,
    pub arch: Arch,
}

impl Target {
    pub(crate) fn detect(
        platform_override: Option<&str>,
        arch_override: Option<&str>,
    ) -> Result<Self> {
        let platform = match platform_override {
            Some(p) => Platform::from_str(p)?,
            None => Platform::detect()?,
        };
        let arch = match arch_override {
            Some(a) => Arch::from_str(a),
            None => Arch::detect(),
        };
        Ok(Self { platform, arch })
    }
}

fn is_rosetta() -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        Command::new("sysctl")
            .args(["-n", "sysctl.proc_translated"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "macos"))]
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_from_str_cases() {
        assert_eq!(Platform::from_str("darwin").unwrap(), Platform::Darwin);
        assert_eq!(Platform::from_str("Darwin").unwrap(), Platform::Darwin);
        assert_eq!(Platform::from_str("macos").unwrap(), Platform::Darwin);
        assert_eq!(Platform::from_str("mac").unwrap(), Platform::Darwin);
        assert_eq!(Platform::from_str("linux").unwrap(), Platform::Linux);
        assert_eq!(Platform::from_str("alpine").unwrap(), Platform::Alpine);
        assert_eq!(Platform::from_str("win32").unwrap(), Platform::Win32);
        assert_eq!(Platform::from_str("windows").unwrap(), Platform::Win32);
        assert_eq!(Platform::from_str("mingw64_nt").unwrap(), Platform::Win32);
        assert!(Platform::from_str("solaris").is_err());
    }

    #[test]
    fn arch_from_str_cases() {
        assert_eq!(Arch::from_str("amd64"), Arch::Amd64);
        assert_eq!(Arch::from_str("arm64"), Arch::Arm64);
        assert_eq!(Arch::from_str("aarch64"), Arch::Arm64);
        // Unknown values fall back to amd64 rather than erroring.
        assert_eq!(Arch::from_str("riscv64"), Arch::Amd64);
        // `x86_64` is amd64 unless running under Rosetta (not the case in tests).
        if !super::is_rosetta() {
            assert_eq!(Arch::from_str("x86_64"), Arch::Amd64);
        }
    }

    // Default platform on Linux is `linux`, even for musl/Alpine targets.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_default_platform_is_linux() {
        let target = Target::detect(None, Some("x86_64")).unwrap();
        assert_eq!(target.platform, Platform::Linux);
        assert_eq!(target.platform.as_str(), "linux");
    }

    // The Alpine/musl artifact is opt-in via `--platform alpine` / FOUNDRYUP_PLATFORM.
    #[test]
    fn alpine_override_still_selects_alpine() {
        let target = Target::detect(Some("alpine"), Some("x86_64")).unwrap();
        assert_eq!(target.platform, Platform::Alpine);
        assert_eq!(target.platform.as_str(), "alpine");
    }
}
