//! The structured, trusted command model for runtime actions.
//!
//! Runtime providers never hand the executor a free-form program string or a
//! shell line. They describe *what* to run as a fully-typed [`ExecutableCommand`]
//! built entirely inside trusted provider code: a program chosen from a closed
//! [`TrustedProgram`] set, OS-native arguments, an explicit environment
//! allowlist, an explicit working directory, a timeout, a [`CommandKind`], and a
//! [`ProcessElevation`] mode. Absolute-path resolution and signature validation
//! happen at execution time, keyed off the `TrustedProgram`, never off caller
//! input. This is the schema-level half of the Runtime Security 1 gate: there is
//! no field through which frontend/API input can name a new executable, inject
//! shell content, or request elevation.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

/// The closed set of executables Studio's runtime providers may invoke.
///
/// Providers name a program by identity, never by a free string and never via a
/// bare `PATH` lookup at call sites. The bare [`name`](TrustedProgram::name) is
/// only a resolution key and a display label; turning it into an absolute,
/// trusted path is the executable-resolution step's job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustedProgram {
    Winget,
    Podman,
    PowerShell,
}

impl TrustedProgram {
    /// The bare program name. Used as the resolution key and for display only —
    /// not as a directly-executed `PATH`-resolved command once trusted
    /// resolution lands.
    pub fn name(self) -> &'static str {
        match self {
            TrustedProgram::Winget => "winget",
            TrustedProgram::Podman => "podman",
            TrustedProgram::PowerShell => "powershell",
        }
    }
}

/// The closed vocabulary of command categories.
///
/// There is deliberately no generic "shell" kind: a provider cannot ask Studio
/// to run arbitrary shell input. Fixed daemon-owned PowerShell scripts that
/// interpolate no user-controlled data are modelled as [`CommandKind::OsConfigTool`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    PackageManager,
    // Part of the closed command vocabulary the Runtime Security gate requires.
    // No current provider runs a downloaded vendor installer executable
    // directly (installs go through the package manager), so it is not
    // constructed yet.
    #[allow(dead_code)]
    VendorInstaller,
    OsConfigTool,
    RuntimeCli,
}

/// Whether a command runs as the current user or through exactly one
/// OS-mediated elevation prompt (UAC).
///
/// Elevation is a fixed property of the trusted command, decided in provider
/// code. It is never requested by the caller and never escalated automatically
/// after an unelevated failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessElevation {
    CurrentUser,
    OneShotOsMediated,
}

/// A single, fully-specified command a runtime provider will run on behalf of
/// an approved action.
///
/// Every field is constructed inside trusted provider code. The only values
/// derived from outside are ones providers read from already-validated
/// persisted profile state (e.g. a machine name), never raw frontend/API input.
pub struct ExecutableCommand {
    pub program: TrustedProgram,
    /// Arguments passed structurally to the process API as OS-native strings.
    /// `OsString` (not `String`) because Windows paths and arguments are not
    /// guaranteed to be valid UTF-8, and the trusted internal representation
    /// must never lose or normalize a valid native argument value. Display-only
    /// previews are built separately as safe UTF-8 summaries, never by
    /// serializing these values.
    pub args: Vec<OsString>,
    /// Environment variable names allowed to pass through to the child process.
    /// Empty means "inherit nothing extra"; secret-bearing parent environment is
    /// never forwarded. Enforced at execution time.
    pub env_allowlist: Vec<&'static str>,
    /// Explicit working directory, or `None` to use the daemon's own.
    pub working_dir: Option<PathBuf>,
    pub timeout: Duration,
    // Read by the display-only preview and audit summary (later Runtime
    // Security 1 steps); constructed by every provider command today.
    #[allow(dead_code)]
    pub kind: CommandKind,
    pub elevation: ProcessElevation,
    /// Human-readable message shown when the command finishes successfully.
    pub success_message: String,
}
