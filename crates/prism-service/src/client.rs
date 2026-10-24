//! What the Prism app uses: requests over the pipe, service state, and
//! running the installer elevated.

use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::ClientOptions;

use crate::{Layout, Request, Response, MAX_MESSAGE, SERVICE_NAME};

/// One request/response round trip with the installed service.
pub async fn request(req: &Request) -> Result<Response> {
    request_at(&Layout::system(), req).await
}

pub async fn request_at(layout: &Layout, req: &Request) -> Result<Response> {
    let pipe = loop {
        match ClientOptions::new().open(&layout.pipe) {
            Ok(p) => break p,
            // ERROR_PIPE_BUSY: all instances in use for a moment.
            Err(e) if e.raw_os_error() == Some(231) => tokio::time::sleep(Duration::from_millis(50)).await,
            Err(e) => return Err(e).context("无法连接 TUN 服务"),
        }
    };
    // Only talk to the installed service, never to a process that grabbed
    // the pipe name first.
    let mut pid = 0u32;
    if unsafe { windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId(pipe.as_raw_handle() as _, &mut pid) } == 0 {
        bail!("无法确认 TUN 服务进程");
    }
    // A normal user can't read a SYSTEM process's image path, so for the
    // real service compare with the PID the service manager reports (only
    // admins can change what PrismService runs).
    let image = process_image(pid);
    let genuine = image.as_deref().is_some_and(|p| same_path(p, &layout.service_exe))
        || (same_path(&layout.service_exe, &crate::service_exe()) && service_pid() == Some(pid));
    if !genuine {
        bail!("{} 不是由 Prism 服务创建的（进程 {pid}，{:?}）", layout.pipe, image);
    }
    let (rd, mut wr) = tokio::io::split(pipe);
    let mut line = serde_json::to_vec(req)?;
    line.push(b'\n');
    wr.write_all(&line).await?;
    wr.flush().await?;
    let mut buf = Vec::new();
    BufReader::new(rd).take(MAX_MESSAGE as u64).read_until(b'\n', &mut buf).await?;
    serde_json::from_slice(&buf).context("TUN 服务返回了无法识别的响应")
}

/// Full path of a process's executable.
pub fn process_image(pid: u32) -> Option<PathBuf> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        (ok != 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
}

/// Windows paths compare case-insensitively.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Missing,
    Stopped,
    Running,
}

/// Process ID of the running PrismService, as the service manager sees it.
pub fn service_pid() -> Option<u32> {
    use windows_service::service::ServiceAccess;
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    let m = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).ok()?;
    m.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS).ok()?.query_status().ok()?.process_id
}

/// Whether the service is installed and running (no admin rights needed).
pub fn state() -> State {
    use windows_service::service::{ServiceAccess, ServiceState};
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    let Ok(m) = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT) else { return State::Missing };
    let Ok(s) = m.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS) else { return State::Missing };
    match s.query_status() {
        Ok(st) if st.current_state == ServiceState::Running => State::Running,
        _ => State::Stopped,
    }
}

/// Runs `exe args` with a UAC prompt and waits for it. Err if the user
/// declined; Ok(exit code) otherwise.
pub fn run_elevated(exe: &Path, args: &str) -> Result<u32> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let verb = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let params = wide(args);
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = SW_HIDE;
        if ShellExecuteExW(&mut info) == 0 || info.hProcess.is_null() {
            bail!("已取消管理员授权");
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        Ok(code)
    }
}

/// The current user's SID, e.g. `S-1-5-21-...`.
pub fn current_user_sid() -> Result<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            bail!("OpenProcessToken failed");
        }
        let mut buf = vec![0u8; 256];
        let mut len = 0u32;
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr() as _, buf.len() as u32, &mut len);
        CloseHandle(token);
        if ok == 0 {
            bail!("GetTokenInformation failed");
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut s) == 0 {
            bail!("ConvertSidToStringSid failed");
        }
        let mut n = 0;
        while *s.add(n) != 0 {
            n += 1;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(s, n));
        LocalFree(s as _);
        Ok(sid)
    }
}
