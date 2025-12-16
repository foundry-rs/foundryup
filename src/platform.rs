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
        if cfg!(target_os = "linux") {
            if is_musl() { Ok(Self::Alpine) } else { Ok(Self::Linux) }
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
            "darwin" | "macos" | "mac" => Ok(Self::Darwin),
            "win32" | "windows" => Ok(Self::Win32),
            s if s.starts_with("mingw") => Ok(Self::Win32),
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
    pub(crate) fn detect() -> Result<Self> {
        let arch = std::env::consts::ARCH;
        match arch {
            "x86_64" => {
                if is_rosetta() {
                    Ok(Self::Arm64)
                } else {
                    Ok(Self::Amd64)
                }
            }
            "aarch64" | "arm64" => Ok(Self::Arm64),
            _ => bail!("unsupported architecture: {arch}"),
        }
    }

    pub(crate) fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "amd64" | "x86_64" | "x64" => Ok(Self::Amd64),
            "arm64" | "aarch64" => Ok(Self::Arm64),
            _ => bail!("unsupported architecture: {s}"),
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
            Some(a) => Arch::from_str(a)?,
            None => Arch::detect()?,
        };
        Ok(Self { platform, arch })
    }
}

fn is_musl() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/etc/os-release")
            .map(|s| s.to_lowercase().contains("alpine"))
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    false
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
