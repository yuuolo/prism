//! What the Prism app uses on macOS: requests over the daemon's unix
//! socket, daemon state, and running the installer as root (through the
//! system's administrator password prompt).

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::{Layout, Request, Response, LAUNCHD_PLIST, MAX_MESSAGE};

/// One request/response round trip with the installed daemon.
pub async fn request(req: &Request) -> Result<Response> {
    request_at(&Layout::system(), req).await
}

pub async fn request_at(layout: &Layout, req: &Request) -> Result<Response> {
    let sock = UnixStream::connect(&layout.pipe).await.context("无法连接 TUN 服务")?;
    // Only talk to the root daemon, never to a process that put a socket
    // there (the folder is root-only, this checks the peer anyway); tests
    // run their own server as the current user.
    let peer = sock.peer_cred().context("无法确认 TUN 服务进程")?;
    let me = unsafe { libc::getuid() };
    if peer.uid() != 0 && !(layout.pipe != crate::PIPE && peer.uid() == me) {
        bail!("{} 不是由 Prism 服务创建的（uid {}）", layout.pipe, peer.uid());
    }
    let (rd, mut wr) = tokio::io::split(sock);
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
    let mut buf = vec![0u8; 4096];
    let n = unsafe { libc::proc_pidpath(pid as libc::c_int, buf.as_mut_ptr() as *mut _, buf.len() as u32) };
    (n > 0).then(|| PathBuf::from(String::from_utf8_lossy(&buf[..n as usize]).into_owned()))
}

/// Same file, following symlinks.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    norm(a) == norm(b)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Missing,
    Stopped,
    Running,
}

/// Whether the daemon is installed and answering (no root needed).
pub fn state() -> State {
    if !Path::new(LAUNCHD_PLIST).exists() {
        return State::Missing;
    }
    match std::os::unix::net::UnixStream::connect(crate::PIPE) {
        Ok(_) => State::Running,
        Err(_) => State::Stopped,
    }
}

/// Quotes a string for `sh`.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Runs `exe args` as root after the system's administrator password prompt
/// and waits for it. `args` is already shell-quoted. Err if the user
/// cancelled; Ok(exit code) otherwise.
pub fn run_elevated(exe: &Path, args: &str) -> Result<u32> {
    let cmd = format!("{} {args}", sh_quote(&exe.to_string_lossy()));
    // AppleScript string: backslashes and quotes escaped.
    let script = format!(
        "do shell script \"{}\" with prompt \"Prism 需要管理员权限来安装 TUN 服务。\" with administrator privileges",
        cmd.replace('\\', "\\\\").replace('"', "\\\"")
    );
    let out = std::process::Command::new("/usr/bin/osascript").arg("-e").arg(script).output().context("run osascript")?;
    if out.status.success() {
        return Ok(0);
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("-128") {
        bail!("已取消管理员授权");
    }
    // "… (N)" at the end carries the command's exit status.
    let code = err.trim().rsplit_once('(').and_then(|(_, c)| c.trim_end_matches(')').parse().ok()).unwrap_or(1);
    Ok(code)
}

/// The current user's uid, as the owner the daemon admits.
pub fn current_user_sid() -> Result<String> {
    Ok(unsafe { libc::getuid() }.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(sh_quote("/Applications/Prism.app/x"), "'/Applications/Prism.app/x'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
    }
}
