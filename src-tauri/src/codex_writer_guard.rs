//! Conservative process gate for destructive writes to native Codex data.
//!
//! A CLI or app-server can write the same files as Desktop. Do not scope this gate to
//! Desktop's private state file, or assume a different process uses a different home.
//! This is a point-in-time check, not a lock against a subsequently launched process.

use crate::error::{AppError, AppResult};
use std::path::Path;

const RUNNING_ERROR: &str = "Codex 原生写入者正在运行；为保护会话数据，已拒绝删除。请完全退出 Codex/ChatGPT 桌面应用、Codex CLI 和 app-server（包括后台进程）后重试";

fn unknown(detail: impl std::fmt::Display) -> AppError {
    AppError::Other(format!(
        "无法安全确认 Codex 原生写入者已停止（{detail}），已拒绝删除"
    ))
}

pub(crate) fn ensure_codex_writers_stopped() -> AppResult<()> {
    ensure_stopped_with(writers_running)
}

/// Local process observations cannot establish quiescence for another machine/WSL host.
/// Resolve aliases first, then ask the OS about the containing volume. Unknown volumes fail shut.
pub(crate) fn ensure_local_source(codex: &Path) -> AppResult<()> {
    if is_unc_source(&codex.to_string_lossy()) {
        return Err(unknown("网络/WSL 数据源须在原生 Agent 所在主机上操作"));
    }
    let resolved = codex.canonicalize().map_err(unknown)?;
    if !resolved.is_dir() || is_unc_source(&resolved.to_string_lossy()) {
        return Err(unknown("数据源不是可确认的本机目录"));
    }
    ensure_local_volume(&resolved)
}

fn is_unc_source(path: &str) -> bool {
    let path = path.replace('\\', "/");
    (path.starts_with("//") && !path.starts_with("//?/"))
        || path.to_ascii_lowercase().starts_with("//?/unc/")
}

#[cfg(windows)]
fn ensure_local_volume(path: &Path) -> AppResult<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumePathNameW};
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut volume = vec![0u16; 32768];
    if unsafe { GetVolumePathNameW(wide.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) } == 0 {
        return Err(unknown(std::io::Error::last_os_error()));
    }
    // DRIVE_REMOVABLE, DRIVE_FIXED, DRIVE_RAMDISK; remote/unknown/nonexistent are refused.
    if !matches!(unsafe { GetDriveTypeW(volume.as_ptr()) }, 2 | 3 | 6) {
        return Err(unknown("数据源位于网络或无法确认的卷，请在来源主机操作"));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ensure_local_volume(path: &Path) -> AppResult<()> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(unknown)?;
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::statfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err(unknown(std::io::Error::last_os_error()));
    }
    let info = unsafe { info.assume_init() };
    #[cfg(target_os = "macos")]
    let local = info.f_flags & libc::MNT_LOCAL != 0;
    // Explicit local filesystem allowlist. FUSE, network, shared VM and overlay filesystems
    // cannot establish which host owns the source and remain unsupported for deletion.
    #[cfg(target_os = "linux")]
    let local = matches!(
        info.f_type as u64,
        0xef53 | 0x58465342 | 0x9123683e | 0x01021994 | 0x858458f6 | 0xf2f52010 | 0x2fc12fc1
    );
    if !local {
        return Err(unknown("数据源位于网络、共享或尚不支持的文件系统"));
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn ensure_local_volume(_path: &Path) -> AppResult<()> {
    Err(unknown("当前平台无法确认本机数据源"))
}

fn ensure_stopped_with(probe: impl FnOnce() -> AppResult<bool>) -> AppResult<()> {
    match probe() {
        Ok(false) => Ok(()),
        Ok(true) => Err(AppError::Other(RUNNING_ERROR.to_owned())),
        Err(error) => Err(unknown(error)),
    }
}

/// Match executable names, never arbitrary command-line arguments. The platform-specific
/// Codex release binaries use a `codex-<target>` name before installation/renaming.
/// ChatGPT is intentionally conservative: some Desktop distributions use that executable.
fn is_writer_executable(executable: &str) -> bool {
    let executable = executable.trim_end_matches(" (deleted)");
    let name = executable.rsplit(['/', '\\']).next().unwrap_or("");
    let name = name.to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    // Linux /proc/<pid>/comm truncates this app-server name to 15 bytes.
    matches!(
        name,
        "codex" | "chatgpt" | "codex-cli" | "codex-app-server" | "codex-app-serve"
    ) || name.starts_with("codex-aarch64-")
        || name.starts_with("codex-x86_64-")
}

fn writers_running() -> AppResult<bool> {
    #[cfg(test)]
    {
        match TEST_PROBES.with_borrow_mut(|probes| probes.pop_front()) {
            Some(TestWriterProbe::Running(running)) => Ok(running),
            Some(TestWriterProbe::Error(message)) => Err(AppError::Other(message.to_owned())),
            None => Ok(false),
        }
    }
    #[cfg(all(not(test), windows))]
    {
        windows_writers_running()
    }
    #[cfg(all(not(test), target_os = "linux"))]
    {
        linux_writers_running()
    }
    #[cfg(all(not(test), target_os = "macos"))]
    {
        macos_writers_running()
    }
    #[cfg(all(not(test), not(any(windows, target_os = "linux", target_os = "macos"))))]
    {
        Err(unknown("当前平台不支持进程检测"))
    }
}

#[cfg(all(not(test), windows))]
fn windows_writers_running() -> AppResult<bool> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    struct Snapshot(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for Snapshot {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(unknown(format!("进程快照失败: {}", unsafe {
            GetLastError()
        })));
    }
    let snapshot = Snapshot(snapshot);
    let mut entry = PROCESSENTRY32W::default();
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    if unsafe { Process32FirstW(snapshot.0, &mut entry) } == 0 {
        // An empty snapshot cannot confirm the current process was enumerated either.
        return Err(unknown(format!("进程枚举失败: {}", unsafe {
            GetLastError()
        })));
    }
    loop {
        let length = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .ok_or_else(|| unknown("进程名称被截断"))?;
        let name = String::from_utf16(&entry.szExeFile[..length])
            .map_err(|_| unknown("进程名称编码无效"))?;
        if is_writer_executable(&name) {
            return Ok(true);
        }
        if unsafe { Process32NextW(snapshot.0, &mut entry) } == 0 {
            let error = unsafe { GetLastError() };
            if error == ERROR_NO_MORE_FILES {
                return Ok(false);
            }
            return Err(unknown(format!("进程枚举失败: {error}")));
        }
    }
}

#[cfg(all(not(test), target_os = "linux"))]
fn linux_writers_running() -> AppResult<bool> {
    let mut saw_self = false;
    let own_pid = std::process::id().to_string();
    for entry in std::fs::read_dir("/proc").map_err(unknown)? {
        let entry = entry.map_err(unknown)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let comm = match std::fs::read_to_string(entry.path().join("comm")) {
            Ok(comm) => comm,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(unknown(error)),
        };
        saw_self |= name == own_pid;
        let comm = comm.trim_end_matches('\n');
        // /proc/comm is limited to 15 bytes. These prefixes also catch release binaries
        // whose names are truncated there. False positives deliberately deny deletion.
        if is_writer_executable(comm) {
            return Ok(true);
        }
    }
    if !saw_self {
        return Err(unknown("进程目录未包含当前进程"));
    }
    Ok(false)
}

#[cfg(all(not(test), target_os = "macos"))]
fn macos_writers_running() -> AppResult<bool> {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    // ps obtains executable metadata, not command lines or session contents. Use the
    // system binary; timeout, excess output or incomplete listings deny deletion.
    const LIMIT: u64 = 4 * 1024 * 1024;
    let mut child = std::process::Command::new("/bin/ps")
        .args(["-Aww", "-o", "pid=", "-o", "comm="])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(unknown)?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let read = |stream: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stream
                .take(LIMIT + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        })
    };
    let out = read(Box::new(stdout));
    let err = read(Box::new(stderr));
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            result => {
                // Only terminate the ps child created by this probe, never an Agent process.
                let _ = child.kill();
                let _ = child.wait();
                break Err(unknown(match result {
                    Err(error) => error.to_string(),
                    _ => "系统进程枚举超时".into(),
                }));
            }
        }
    };
    let stdout = out
        .join()
        .map_err(|_| unknown("进程枚举读取失败"))?
        .map_err(unknown)?;
    let stderr = err
        .join()
        .map_err(|_| unknown("进程枚举读取失败"))?
        .map_err(unknown)?;
    if !status?.success() || !stderr.is_empty() || stdout.len() > LIMIT as usize {
        return Err(unknown("系统进程枚举失败"));
    }
    let listing = std::str::from_utf8(&stdout).map_err(unknown)?;
    parse_process_listing(listing, std::process::id())
}

#[cfg(any(target_os = "macos", test))]
fn parse_process_listing(listing: &str, own_pid: u32) -> AppResult<bool> {
    let mut saw_self = false;
    let mut running = false;
    for line in listing.lines() {
        let line = line.trim_start();
        let (pid, executable) = line
            .split_once(char::is_whitespace)
            .ok_or_else(|| unknown("进程枚举输出不完整"))?;
        let pid = pid.parse::<u32>().map_err(|_| unknown("进程编号无效"))?;
        let executable = executable.trim_start();
        if executable.is_empty() {
            return Err(unknown("进程可执行文件信息缺失"));
        }
        saw_self |= pid == own_pid;
        running |= is_writer_executable(executable);
    }
    if !saw_self {
        return Err(unknown("进程枚举未包含当前进程"));
    }
    Ok(running)
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) enum TestWriterProbe {
    Running(bool),
    Error(&'static str),
}

#[cfg(test)]
thread_local! {
    static TEST_PROBES: std::cell::RefCell<std::collections::VecDeque<TestWriterProbe>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

/// Scoped, thread-local injection; absent from production builds.
#[cfg(test)]
pub(crate) struct WriterTestProbeGuard(std::collections::VecDeque<TestWriterProbe>);

#[cfg(test)]
impl WriterTestProbeGuard {
    pub(crate) fn running() -> Self {
        Self::sequence([TestWriterProbe::Running(true)])
    }

    pub(crate) fn running_after_not_running(count: usize) -> Self {
        Self::sequence(
            std::iter::repeat(TestWriterProbe::Running(false))
                .take(count)
                .chain([TestWriterProbe::Running(true)]),
        )
    }

    pub(crate) fn sequence(probes: impl IntoIterator<Item = TestWriterProbe>) -> Self {
        Self(TEST_PROBES.replace(probes.into_iter().collect()))
    }
}

#[cfg(test)]
impl Drop for WriterTestProbeGuard {
    fn drop(&mut self) {
        TEST_PROBES.set(std::mem::take(&mut self.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_desktop_cli_and_app_server_executables() {
        for executable in [
            "/Applications/Codex.app/Contents/MacOS/Codex",
            "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT",
            "C:\\Program Files\\Codex\\ChatGPT.exe",
            "CODEX.EXE",
            "/usr/bin/codex",
            "/opt/codex-app-server",
            "codex-app-serve",
            "/usr/local/bin/codex-aarch64-apple-darwin",
            "codex-x86_64-unknown-linux-musl",
            "codex-aarch64-u",
            "codex-x86_64-un",
            "/usr/bin/codex (deleted)",
        ] {
            assert!(is_writer_executable(executable), "{executable}");
        }
        for executable in [
            "agentvault",
            "cc-sessions",
            "code",
            "codex-notes.txt",
            "/tmp/codex/editor",
        ] {
            assert!(!is_writer_executable(executable), "{executable}");
        }
    }

    #[test]
    fn malformed_or_incomplete_listing_is_not_stopped() {
        for listing in ["", "7", "bad /bin/editor", "7 ", "8 /bin/editor"] {
            assert!(parse_process_listing(listing, 7).is_err());
        }
        assert!(!parse_process_listing("7 /bin/agentvault\n8 /bin/editor\n", 7).unwrap());
        assert!(parse_process_listing(
            "7 /bin/agentvault\n8 /Applications/Codex.app/Contents/MacOS/Codex\n",
            7
        )
        .unwrap());
    }

    #[test]
    fn running_and_unknown_both_refuse_deletion() {
        assert!(ensure_stopped_with(|| Ok(false)).is_ok());
        assert!(ensure_stopped_with(|| Ok(true))
            .unwrap_err()
            .to_string()
            .contains("已拒绝删除"));
        assert!(
            ensure_stopped_with(|| Err(AppError::Other("permission denied".into())))
                .unwrap_err()
                .to_string()
                .contains("已拒绝删除")
        );
    }

    #[test]
    fn scoped_probe_restores_previous_state() {
        let _outer = WriterTestProbeGuard::running();
        {
            let _inner = WriterTestProbeGuard::sequence([TestWriterProbe::Error("denied")]);
            assert!(ensure_codex_writers_stopped()
                .unwrap_err()
                .to_string()
                .contains("denied"));
        }
        assert!(ensure_codex_writers_stopped().is_err());
    }

    #[test]
    fn transition_to_running_is_injectable() {
        let _probe = WriterTestProbeGuard::running_after_not_running(2);
        assert!(ensure_codex_writers_stopped().is_ok());
        assert!(ensure_codex_writers_stopped().is_ok());
        assert!(ensure_codex_writers_stopped().is_err());
    }

    #[test]
    fn remote_and_wsl_roots_are_rejected_before_access() {
        for path in [
            r"\\mac\Home\.codex",
            r"\\wsl.localhost\Ubuntu\home\user\.codex",
            r"\\?\UNC\server\share\.codex",
            "//server/share/.codex",
        ] {
            assert!(is_unc_source(path));
            assert!(ensure_local_source(Path::new(path)).is_err());
        }
        assert!(!is_unc_source(r"\\?\C:\Users\test\.codex"));
        assert!(!is_unc_source("/home/test/.codex"));
    }
}
