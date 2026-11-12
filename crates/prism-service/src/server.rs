//! The service side: the pipe (Windows) / unix socket (macOS) server and
//! the core it runs.

use std::collections::{BTreeMap, HashMap, VecDeque};
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
#[cfg(windows)]
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
    #[cfg(windows)]
    kill_switch: Mutex<Option<crate::killswitch::KillSwitch>>,
    #[cfg(windows)]
    tail: Mutex<crate::tailguard::Control>,
    #[cfg(windows)]
    tail_identity: Mutex<Option<Arc<crate::tailguard::Identity>>>,
    #[cfg(windows)]
    tail_refresh: tokio::sync::Mutex<()>,
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
    /// Only the cores this service runs may connect outside the TUN.
    #[cfg(windows)]
    fn set_kill_switch(&self, on: bool) -> Result<()> {
        let mut ks = self.kill_switch.lock().unwrap();
        match (on, ks.is_some()) {
            (true, false) => {
                let allowed: Vec<PathBuf> = CORES.iter().map(|(_, file)| self.layout.cores_dir().join(file)).filter(|p| p.exists()).collect();
                *ks = Some(crate::killswitch::KillSwitch::enable(&allowed).context("启用断网保护失败")?);
                push_log(&self.logs, "[service] 断网保护已启用".into());
            }
            (false, true) => {
                *ks = None;
                push_log(&self.logs, "[service] 断网保护已解除".into());
            }
            _ => {}
        }
        Ok(())
    }

    #[cfg(not(windows))]
    fn set_kill_switch(&self, on: bool) -> Result<()> {
        if on {
            bail!("断网保护目前只支持 Windows");
        }
        Ok(())
    }

    #[cfg(windows)]
    fn kill_switch_on(&self) -> bool {
        self.kill_switch.lock().unwrap().is_some()
    }

    #[cfg(not(windows))]
    fn kill_switch_on(&self) -> bool {
        false
    }

    #[cfg(windows)]
    async fn refresh_tail(&self) -> Result<()> {
        use crate::tailguard::{self, Permit};
        let _guard=self.tail_refresh.lock().await;
        let (requested,suspended,lease,generation)={let t=self.tail.lock().unwrap();(t.requested,t.suspended,t.lease_left(std::time::Instant::now()),t.generation)};
        let protected=self.kill_switch_on();
        let mut status=TailscaleProtection{requested,..Default::default()};
        let mut permit=None;
        if requested && protected && !suspended {
            let cached=self.tail_identity.lock().unwrap().clone();
            let identity=tokio::task::spawn_blocking(move ||tailguard::identity(cached)).await.unwrap_or_else(|e|Err(anyhow!("身份验证任务失败：{e}")));
            match identity {
                Ok(id)=>{
                    *self.tail_identity.lock().unwrap()=Some(id.clone()); status.verified=true;
                    match tailguard::connected(id.clone()).await {
                        Ok(p)=>{status.connected=true;status.phase="connected".into();status.interface_index=p.interface.as_ref().map(|i|i.index);status.peer_addresses=p.peers.len();status.detail="已验证守护进程及组网接口，限地址放行".into();permit=Some(p);let mut t=self.tail.lock().unwrap();if t.generation==generation {t.lease_until=None;}}
                        Err(e) if lease>0=>{status.phase="connecting".into();status.detail=format!("短期连接授权：{e:#}");status.lease_seconds=lease;permit=Some(Permit{identity:id,interface:None,peers:vec![]});}
                        Err(e)=>{status.phase="disconnected".into();status.detail=format!("组网例外已撤销：{e:#}");}
                    }
                }
                Err(e)=>{*self.tail_identity.lock().unwrap()=None;status.phase="unverified".into();status.detail=format!("未放行：{e:#}");}
            }
        } else {status.phase=if suspended {"suspended"} else {"disabled"}.into();status.detail="无组网例外，互联网保护策略保持不变".into();}
        let current=self.tail.lock().unwrap();
        let live_requested=current.requested; status.requested=live_requested;
        if status.phase=="connecting" {status.lease_seconds=current.lease_left(std::time::Instant::now());}
        if current.generation!=generation || status.phase=="connecting" && current.lease_left(std::time::Instant::now())==0 {
            permit=None;status.connected=false;status.phase="revoked".into();status.detail="授权状态已变更或连接授权已过期，取消旧例外".into();status.lease_seconds=0;
        }
        drop(current);
        // Keep the old binary handles alive until all corresponding filters
        // have actually been removed. Scope blocks remain during replacement.
        if let Some(ks)=self.kill_switch.lock().unwrap().as_mut() {
            if let Err(e)=ks.set_tailnet(live_requested,permit) {
                self.tail.lock().unwrap().status=TailscaleProtection{requested:live_requested,phase:"error".into(),detail:format!("组网规则更新/撤销失败，基础断网保护仍保留，正在重试：{e:#}"),..Default::default()};
                return Err(e);
            }
        } else {
            status.connected=false;status.phase="disabled".into();status.interface_index=None;status.peer_addresses=0;status.lease_seconds=0;
            status.detail="断网保护未启用，无需添加组网例外".into();
        }
        let changed={let old=&self.tail.lock().unwrap().status; old.phase!=status.phase || old.interface_index!=status.interface_index || old.peer_addresses!=status.peer_addresses};
        if !status.verified {*self.tail_identity.lock().unwrap()=None;}
        if changed {push_log(&self.logs,format!("[service] Tailscale 保护 {}：{}",status.phase,status.detail));}
        self.tail.lock().unwrap().status=status;
        Ok(())
    }

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
                push_log(&self.logs, format!("[service] {} 内核异常退出{}", r.kind,
                    if self.kill_switch_on() { "，断网保护已触发" } else { "" }));
                *core = None;
            }
        }
        let mut st = Status {
            running: core.is_some(),
            core: core.as_ref().map(|r| r.kind.clone()),
            pid: core.as_ref().and_then(|r| r.child.id()),
            version: env!("CARGO_PKG_VERSION").into(),
            wintun: self.hash(&self.layout.wintun()),
            kill_switch: self.kill_switch_on(),
            tailscale_supported: cfg!(windows),
            #[cfg(windows)]
            tailscale: self.tail.lock().unwrap().status.clone(),
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

    async fn stop_core(&self) {
        if let Some(mut r) = self.core.lock().await.take() {
            let _ = r.child.kill().await;
            push_log(&self.logs, format!("[service] {} 已停止", r.kind));
        }
    }

    /// Stops on request: the user wants the network back, so the block goes too.
    async fn stop(&self) {
        #[cfg(windows)] self.tail.lock().unwrap().request(false);
        self.stop_core().await;
        if let Err(e) = self.set_kill_switch(false) {
            log(&self.layout, &format!("{e:#}"));
        }
    }

    async fn start(&self, kind: &str, config: &str, files: &BTreeMap<String, String>, kill_switch: bool, tailscale: bool) -> Result<()> {
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
        #[cfg(windows)] self.tail.lock().unwrap().request(tailscale && kind=="prism");
        #[cfg(not(windows))] let _=tailscale;
        // Before the old core goes, so a restart has no unprotected moment.
        if kill_switch {
            self.set_kill_switch(true)?;
            #[cfg(windows)] self.refresh_tail().await?;
        }
        self.stop_core().await;
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
        if !kill_switch {
            self.set_kill_switch(false)?;
        }
        Ok(())
    }

    fn logs_since(&self, since: u64) -> Vec<(u64, String)> {
        self.logs.lock().unwrap().lines.iter().filter(|(n, _)| *n > since).cloned().collect()
    }
}

/// A pipe instance only the owner (plus SYSTEM / admins) can open.
#[cfg(windows)]
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

fn load_state(layout: Layout) -> Result<Arc<State>> {
    let owner: Owner = serde_json::from_slice(&std::fs::read(layout.owner_file()).context("service.json missing, reinstall the service")?)?;
    Ok(Arc::new(State {
        layout,
        owner,
        core: Default::default(),
        #[cfg(windows)]
        kill_switch: Default::default(),
        #[cfg(windows)] tail: Default::default(),
        #[cfg(windows)] tail_identity: Default::default(),
        #[cfg(windows)] tail_refresh: Default::default(),
        logs: Default::default(),
        hashes: Default::default(),
    }))
}

/// Serves requests until `stop` turns true, then stops the core.
#[cfg(windows)]
pub async fn serve(layout: Layout, mut stop: tokio::sync::watch::Receiver<bool>) -> Result<()> {
    let pipe_name = layout.pipe.clone();
    let state = load_state(layout)?;
    let sid = state.owner.user_sid.clone();
    let mut pipe = create_pipe(&pipe_name, &sid, true)?;
    let weak=Arc::downgrade(&state); let mut tail_stop=stop.clone();
    tokio::spawn(async move {
        loop { tokio::select! {
            _=tail_stop.changed()=>break,
            _=tokio::time::sleep(std::time::Duration::from_secs(2))=>{
                let Some(st)=weak.upgrade() else {break};
                if let Err(e)=st.refresh_tail().await {push_log(&st.logs,format!("[service] Tailscale 保护更新失败：{e:#}"));}
            }
        }}
    });
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

#[cfg(windows)]
async fn handle(conn: NamedPipeServer, st: &State) -> Result<()> {
    let mut pid = 0u32;
    if unsafe { windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId(conn.as_raw_handle() as _, &mut pid) } == 0 {
        bail!("no client pid");
    }
    let image = process_image(pid).ok_or_else(|| anyhow!("client {pid} image unknown"))?;
    let allowed = same_path(&image, &st.owner.gui_path);
    answer(conn, &image, allowed, st).await
}

/// Serves requests until `stop` turns true, then stops the core. The
/// socket is open to everyone; each connection is checked by its peer's
/// uid and executable.
#[cfg(unix)]
pub async fn serve(layout: Layout, mut stop: tokio::sync::watch::Receiver<bool>) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let path = layout.pipe.clone();
    let state = load_state(layout)?;
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path).with_context(|| format!("listen on {path}"))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666))?;
    loop {
        tokio::select! {
            r = listener.accept() => {
                let (conn, _) = r?;
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
    let _ = std::fs::remove_file(&path);
    Ok(())
}

#[cfg(unix)]
async fn handle(conn: tokio::net::UnixStream, st: &State) -> Result<()> {
    let cred = conn.peer_cred()?;
    let pid = cred.pid().ok_or_else(|| anyhow!("no client pid"))?;
    let image = process_image(pid as u32).ok_or_else(|| anyhow!("client {pid} image unknown"))?;
    let allowed = cred.uid().to_string() == st.owner.user_sid && same_path(&image, &st.owner.gui_path);
    answer(conn, &image, allowed, st).await
}

/// Reads one request and answers it, or refuses a client that isn't the
/// registered Prism.
async fn answer<S: tokio::io::AsyncRead + tokio::io::AsyncWrite>(conn: S, image: &Path, allowed: bool, st: &State) -> Result<()> {
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
        Request::TailscaleConnect {connect} => {
            #[cfg(windows)] {
                st.tail.lock().unwrap().connect(connect,std::time::Instant::now());
                match st.refresh_tail().await {Ok(())=>Response{ok:true,status:Some(st.status().await),..Default::default()},Err(e)=>Response::err(e)}
            }
            #[cfg(not(windows))] {let _=connect; Response::err("此保护联动目前只支持 Windows")}
        }
        Request::Logs { since } => Response { ok: true, lines: st.logs_since(since), ..Default::default() },
        Request::Start { core, config, files, kill_switch, tailscale } => match st.start(&core, &config, &files, kill_switch, tailscale).await {
            Ok(()) => Response { ok: true, status: Some(st.status().await), ..Default::default() },
            Err(e) => Response::err(e),
        },
    }
}
