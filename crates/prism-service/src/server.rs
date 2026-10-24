//! The service side: the pipe server and the core it runs.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::process::{Child, Command};

use crate::client::{process_image, same_path};
use crate::*;

const MAX_LOG_LINES: usize = 2000;

struct Running {
    kind: String,
    child: Child,
}

#[derive(Default)]
struct Logs {
    next: u64,
    lines: VecDeque<(u64, String)>,
}

struct State {
    layout: Layout,
    owner: Owner,
    core: tokio::sync::Mutex<Option<Running>>,
    logs: Arc<Mutex<Logs>>,
    /// path -> (len, mtime, sha256)
    hashes: Mutex<HashMap<PathBuf, (u64, SystemTime, String)>>,
}

fn push_log(logs: &Mutex<Logs>, line: String) {
    let mut l = logs.lock().unwrap();
    l.next += 1;
    let n = l.next;
    l.lines.push_back((n, line));
    while l.lines.len() > MAX_LOG_LINES {
        l.lines.pop_front();
    }
}

impl State {
    fn hash(&self, p: &Path) -> Option<String> {
        let meta = p.metadata().ok()?;
        let key = (meta.len(), meta.modified().ok()?);
        if let Some((len, t, h)) = self.hashes.lock().unwrap().get(p) {
            if (*len, *t) == key {
                return Some(h.clone());
            }
        }
        let h = sha256_file(p)?;
        self.hashes.lock().unwrap().insert(p.to_path_buf(), (key.0, key.1, h.clone()));
        Some(h)
    }

    async fn status(&self) -> Status {
        let mut core = self.core.lock().await;
        if let Some(r) = core.as_mut() {
            if !matches!(r.child.try_wait(), Ok(None)) {
                *core = None;
            }
        }
        let mut st = Status {
            running: core.is_some(),
            core: core.as_ref().map(|r| r.kind.clone()),
            pid: core.as_ref().and_then(|r| r.child.id()),
            version: env!("CARGO_PKG_VERSION").into(),
            wintun: self.hash(&self.layout.wintun()),
            ..Default::default()
        };
        drop(core);
        for (kind, file) in CORES {
            if let Some(h) = self.hash(&self.layout.cores_dir().join(file)) {
                st.cores.insert(kind.into(), h);
            }
        }
        let run = self.layout.run_dir();
        let list = |dir: PathBuf, prefix: &str| -> Vec<String> {
            std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| format!("{prefix}{}", e.file_name().to_string_lossy()))
                .collect()
        };
        let mut names = list(run.clone(), "");
        names.extend(list(run.join("rule-set"), "rule-set/"));
        for n in names.into_iter().filter(|n| allowed_file(n)) {
            if let Some(h) = self.hash(&run.join(&n)) {
                st.files.insert(n, h);
            }
        }
        st
    }

    async fn stop(&self) {
        if let Some(mut r) = self.core.lock().await.take() {
            let _ = r.child.kill().await;
            push_log(&self.logs, format!("[service] {} 已停止", r.kind));
        }
    }

    async fn start(&self, kind: &str, config: &str, files: &BTreeMap<String, String>) -> Result<()> {
        let file = core_file(kind).with_context(|| format!("unknown core {kind}"))?;
        let bin = self.layout.cores_dir().join(file);
        if !bin.exists() {
            bail!("服务中没有 {kind} 内核，请在 Prism 中更新 TUN 服务");
        }
        // Check everything before touching the running core.
        for name in files.keys() {
            if !allowed_file(name) {
                bail!("file {name:?} is not allowed");
            }
        }
        let config = sanitize_config(kind, config)?;
        self.stop().await;
        let run = self.layout.run_dir();
        std::fs::create_dir_all(run.join("rule-set"))?;
        for (name, data) in files {
            let bytes = base64::engine::general_purpose::STANDARD.decode(data).context("bad file data")?;
            std::fs::write(run.join(name), bytes)?;
        }
        if kind == "prism" && self.layout.wintun().exists() {
            let dll = run.join("wintun.dll");
            if self.hash(&dll) != self.hash(&self.layout.wintun()) {
                std::fs::copy(self.layout.wintun(), &dll)?;
            }
        }
        let cfg = run.join(config_file(kind));
        std::fs::write(&cfg, config)?;

        let mut cmd = Command::new(&bin);
        if kind == "singbox" {
            cmd.arg("run").arg("-c").arg(&cfg).arg("-D").arg(&run).arg("--disable-color");
        } else {
            cmd.arg("-f").arg(&cfg).arg("-d").arg(&run);
        }
        cmd.current_dir(&run).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        let mut child = cmd.spawn().with_context(|| format!("start {}", bin.display()))?;
        push_log(&self.logs, format!("[service] 以系统服务启动 {kind} 内核（pid {}）", child.id().unwrap_or(0)));
        for out in [
            child.stdout.take().map(|o| Box::new(o) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
            child.stderr.take().map(|e| Box::new(e) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let logs = self.logs.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(out).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    push_log(&logs, line);
                }
            });
        }
        *self.core.lock().await = Some(Running { kind: kind.into(), child });
        Ok(())
    }

    fn logs_since(&self, since: u64) -> Vec<(u64, String)> {
        self.logs.lock().unwrap().lines.iter().filter(|(n, _)| *n > since).cloned().collect()
    }
}

/// A pipe instance only the owner (plus SYSTEM / admins) can open.
fn create_pipe(name: &str, sid: &str, first: bool) -> Result<NamedPipeServer> {
    use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    let sddl: Vec<u16> = format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;{sid})").encode_utf16().chain(Some(0)).collect();
    let mut sd = std::ptr::null_mut();
    if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) } == 0 {
        bail!("bad security descriptor for {sid}");
    }
    let mut sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
    let pipe = unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(name, &mut sa as *mut _ as *mut _)
    };
    unsafe { windows_sys::Win32::Foundation::LocalFree(sd) };
    Ok(pipe?)
}

/// Serves requests until `stop` turns true, then stops the core.
pub async fn serve(layout: Layout, mut stop: tokio::sync::watch::Receiver<bool>) -> Result<()> {
    let owner: Owner = serde_json::from_slice(&std::fs::read(layout.owner_file()).context("service.json missing, reinstall the service")?)?;
    let sid = owner.user_sid.clone();
    let pipe_name = layout.pipe.clone();
    let state = Arc::new(State { layout, owner, core: Default::default(), logs: Default::default(), hashes: Default::default() });
    let mut pipe = create_pipe(&pipe_name, &sid, true)?;
    loop {
        tokio::select! {
            r = pipe.connect() => {
                r?;
                let conn = std::mem::replace(&mut pipe, create_pipe(&pipe_name, &sid, false)?);
                let st = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle(conn, &st).await {
                        log(&st.layout, &format!("request failed: {e:#}"));
                    }
                });
            }
            _ = stop.changed() => break,
        }
    }
    state.stop().await;
    Ok(())
}

async fn handle(conn: NamedPipeServer, st: &State) -> Result<()> {
    let mut pid = 0u32;
    if unsafe { windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId(conn.as_raw_handle() as _, &mut pid) } == 0 {
        bail!("no client pid");
    }
    let image = process_image(pid).ok_or_else(|| anyhow!("client {pid} image unknown"))?;
    let allowed = same_path(&image, &st.owner.gui_path);
    let (rd, mut wr) = tokio::io::split(conn);
    // Read the request even when refusing (only a little of it), so the
    // client's write doesn't hit a pipe we already closed.
    let limit = if allowed { MAX_MESSAGE as u64 } else { 64 << 10 };
    let mut buf = Vec::new();
    BufReader::new(rd).take(limit).read_until(b'\n', &mut buf).await?;
    let reply = if !allowed {
        log(&st.layout, &format!("rejected client {}", image.display()));
        Response::err(format!("{} 不是安装服务时登记的 Prism", image.display()))
    } else {
        match serde_json::from_slice::<Request>(&buf) {
            Ok(req) => dispatch(req, st).await,
            Err(e) => Response::err(format!("bad request: {e}")),
        }
    };
    let mut out = serde_json::to_vec(&reply)?;
    out.push(b'\n');
    wr.write_all(&out).await?;
    wr.flush().await?;
    Ok(())
}

async fn dispatch(req: Request, st: &State) -> Response {
    match req {
        Request::Status => Response { ok: true, status: Some(st.status().await), ..Default::default() },
        Request::Stop => {
            st.stop().await;
            Response { ok: true, ..Default::default() }
        }
        Request::Logs { since } => Response { ok: true, lines: st.logs_since(since), ..Default::default() },
        Request::Start { core, config, files } => match st.start(&core, &config, &files).await {
            Ok(()) => Response { ok: true, status: Some(st.status().await), ..Default::default() },
            Err(e) => Response::err(e),
        },
    }
}
