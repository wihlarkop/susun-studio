//! Exact one-shot Windows elevation with no shell intermediary.

use std::{ffi::OsString, path::Path, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevatedProcessOutcome {
    Exited(u32),
    Cancelled,
    TimedOut,
}

#[derive(Debug, thiserror::Error)]
pub enum ElevationError {
    #[error("OS-mediated elevation is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("OS-mediated elevation could not start the approved command")]
    LaunchFailed,
    #[error("OS-mediated elevation returned no process handle")]
    MissingProcess,
    #[error("elevated process exit status was unavailable")]
    ExitCodeUnavailable,
}

#[cfg(windows)]
pub fn run_elevated_process(
    executable: &Path,
    args: &[OsString],
    working_dir: Option<&Path>,
    timeout: Duration,
) -> Result<ElevatedProcessOutcome, ElevationError> {
    imp::run(executable, args, working_dir, timeout)
}

#[cfg(not(windows))]
pub fn run_elevated_process(
    _executable: &Path,
    _args: &[OsString],
    _working_dir: Option<&Path>,
    _timeout: Duration,
) -> Result<ElevatedProcessOutcome, ElevationError> {
    Err(ElevationError::UnsupportedPlatform)
}

#[cfg(windows)]
mod imp {
    #![allow(unsafe_code)]

    use super::*;
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
    use windows::{
        Win32::{
            Foundation::{CloseHandle, ERROR_CANCELLED, GetLastError, WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            },
            System::Threading::{GetExitCodeProcess, TerminateProcess, WaitForSingleObject},
            UI::{
                Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
                WindowsAndMessaging::SW_HIDE,
            },
        },
        core::PCWSTR,
    };

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn parameters(args: &[OsString]) -> OsString {
        args.iter()
            .map(|arg| quote(arg.as_os_str()))
            .collect::<Vec<_>>()
            .join(OsStr::new(" "))
    }

    fn quote(arg: &OsStr) -> OsString {
        let value = arg.to_string_lossy();
        if !value.is_empty() && !value.chars().any(|ch| ch.is_whitespace() || ch == '"') {
            return arg.to_os_string();
        }
        let mut result = '"'.to_string();
        let mut slashes = 0;
        for ch in value.chars() {
            match ch {
                '\\' => slashes += 1,
                '"' => {
                    result.push_str(&"\\".repeat(slashes * 2 + 1));
                    result.push('"');
                    slashes = 0;
                }
                _ => {
                    result.push_str(&"\\".repeat(slashes));
                    slashes = 0;
                    result.push(ch);
                }
            }
        }
        result.push_str(&"\\".repeat(slashes * 2));
        result.push('"');
        result.into()
    }

    pub fn run(
        executable: &Path,
        args: &[OsString],
        working_dir: Option<&Path>,
        timeout: Duration,
    ) -> Result<ElevatedProcessOutcome, ElevationError> {
        // The job owns exactly this approved launch and its descendants. Closing
        // it on timeout applies KILL_ON_JOB_CLOSE to the whole created tree.
        let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
            .map_err(|_| ElevationError::LaunchFailed)?;
        let mut job_info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        job_info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&job_info).cast(),
                u32::try_from(std::mem::size_of_val(&job_info)).unwrap_or(u32::MAX),
            )
        };
        if configured.is_err() {
            let _ = unsafe { CloseHandle(job) };
            return Err(ElevationError::LaunchFailed);
        }

        let verb = wide(OsStr::new("runas"));
        let file = wide(executable.as_os_str());
        let params = wide(parameters(args).as_os_str());
        let directory = working_dir.map(|path| wide(path.as_os_str()));
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(params.as_ptr()),
            lpDirectory: directory
                .as_ref()
                .map_or(PCWSTR::null(), |value| PCWSTR(value.as_ptr())),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        // SAFETY: all pointers reference live NUL-terminated buffers.
        if unsafe { ShellExecuteExW(&mut info) }.is_err() {
            let _ = unsafe { CloseHandle(job) };
            // SAFETY: read immediately after the failed Win32 call.
            return if unsafe { GetLastError() } == ERROR_CANCELLED {
                Ok(ElevatedProcessOutcome::Cancelled)
            } else {
                Err(ElevationError::LaunchFailed)
            };
        }
        if info.hProcess.is_invalid() {
            let _ = unsafe { CloseHandle(job) };
            return Err(ElevationError::MissingProcess);
        }
        if unsafe { AssignProcessToJobObject(job, info.hProcess) }.is_err() {
            let _ = unsafe { TerminateProcess(info.hProcess, 1) };
            let _ = unsafe { CloseHandle(info.hProcess) };
            let _ = unsafe { CloseHandle(job) };
            return Err(ElevationError::LaunchFailed);
        }
        let wait_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: hProcess is owned and live until CloseHandle below.
        let wait = unsafe { WaitForSingleObject(info.hProcess, wait_ms) };
        let outcome = if wait == WAIT_TIMEOUT {
            // SAFETY: closing this private job terminates only the approved
            // process tree because KILL_ON_JOB_CLOSE was configured above.
            let _ = unsafe { CloseHandle(job) };
            let _ = unsafe { WaitForSingleObject(info.hProcess, 5_000) };
            Ok(ElevatedProcessOutcome::TimedOut)
        } else if wait == WAIT_OBJECT_0 {
            let mut code = 0;
            // SAFETY: process handle remains valid.
            unsafe { GetExitCodeProcess(info.hProcess, &mut code) }
                .map_err(|_| ElevationError::ExitCodeUnavailable)?;
            Ok(ElevatedProcessOutcome::Exited(code))
        } else {
            Err(ElevationError::ExitCodeUnavailable)
        };
        if wait != WAIT_TIMEOUT {
            let _ = unsafe { CloseHandle(job) };
        }
        // SAFETY: closes the one owned process handle exactly once.
        let _ = unsafe { CloseHandle(info.hProcess) };
        outcome
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parameters_preserve_metacharacters_as_argument_data() {
            let args = vec![
                OsString::from("plain"),
                OsString::from("two words"),
                OsString::from("value;&|$()"),
                OsString::from("quote\"and\\slash"),
            ];
            assert_eq!(
                parameters(&args),
                OsString::from("plain \"two words\" value;&|$() \"quote\\\"and\\slash\"")
            );
        }
    }
}
