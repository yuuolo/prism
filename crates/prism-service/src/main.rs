//! prism-service.exe
//!   install --user <SID> --gui <prism.exe> --wintun <dll> --core <kind>=<exe>...
//!                 copy into %ProgramFiles%\Prism Service and (re)start the
//!                 service (run elevated; also used to refresh the cores)
//!   uninstall     stop and remove the service and its folders
//!   run           service entry point (started by Windows)

#[cfg(not(windows))]
fn main() {
    eprintln!("prism-service only runs on Windows");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let r = match args.get(1).map(String::as_str) {
        Some("run") => win::run_service(),
        Some("install") => win::install(&args[2..]),
        Some("uninstall") => win::uninstall(),
        _ => {
            eprintln!("usage: prism-service install --user <SID> --gui <path> --wintun <dll> --core <kind>=<exe>... | uninstall");
            std::process::exit(2);
        }
    };
    if let Err(e) = r {
        prism_service::log(&prism_service::Layout::system(), &format!("error: {e:#}"));
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod win {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::process::Stdio;
    use std::time::Duration;

    use anyhow::{bail, Context, Result};
    use prism_service::client::same_path;
    use prism_service::*;
    use windows_service::service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode, ServiceInfo, ServiceStartType, ServiceState,
        ServiceStatus, ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    use windows_service::{define_windows_service, service_dispatcher};

    fn arg_values<'a>(args: &'a [String], name: &str) -> Vec<&'a str> {
        args.windows(2).filter(|w| w[0] == name).map(|w| w[1].as_str()).collect()
    }

    fn valid_sid(s: &str) -> bool {
        s.starts_with("S-1-") && s.len() < 200 && s[4..].chars().all(|c| c.is_ascii_digit() || c == '-')
    }

    fn manager(access: ServiceManagerAccess) -> Result<ServiceManager> {
        ServiceManager::local_computer(None::<&str>, access).context("open service manager (needs administrator)")
    }

    fn stop_and_wait(svc: &windows_service::service::Service) {
        if let Ok(s) = svc.query_status() {
            if s.current_state != ServiceState::Stopped {
                let _ = svc.stop();
            }
        }
        for _ in 0..100 {
            match svc.query_status() {
                Ok(s) if s.current_state != ServiceState::Stopped => std::thread::sleep(Duration::from_millis(100)),
                _ => break,
            }
        }
    }

    /// The service reports STOPPED a little before its process is gone.
    fn wait_exit(pid: u32) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
        unsafe {
            let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if !h.is_null() {
                WaitForSingleObject(h, 10_000);
                CloseHandle(h);
            }
        }
    }

    fn copy_file(from: &Path, to: &Path) -> Result<()> {
        if same_path(from, to) {
            return Ok(());
        }
        // A file still held by an exiting process frees up shortly.
        let mut tries = 0;
        loop {
            match std::fs::copy(from, to) {
                Ok(_) => return Ok(()),
                Err(e) if e.raw_os_error() == Some(32) && tries < 20 => {
                    tries += 1;
                    std::thread::sleep(Duration::from_millis(250));
                }
                Err(e) => return Err(e).with_context(|| format!("copy {} -> {}", from.display(), to.display())),
            }
        }
    }

    pub fn install(args: &[String]) -> Result<()> {
        let layout = Layout::system();
        let sid = *arg_values(args, "--user").first().context("--user <SID> missing")?;
        if !valid_sid(sid) {
            bail!("bad SID {sid}");
        }
        let gui = PathBuf::from(*arg_values(args, "--gui").first().context("--gui missing")?);
        let wintun = PathBuf::from(*arg_values(args, "--wintun").first().context("--wintun missing")?);
        let mut cores = Vec::new();
        for c in arg_values(args, "--core") {
            let (kind, path) = c.split_once('=').context("--core <kind>=<exe>")?;
            let file = core_file(kind).with_context(|| format!("unknown core {kind}"))?;
            cores.push((file, PathBuf::from(path)));
        }
        if cores.is_empty() {
            bail!("no --core given");
        }

        let m = manager(ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)?;
        let access = ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::STOP | ServiceAccess::CHANGE_CONFIG;
        let existing = m.open_service(SERVICE_NAME, access).ok();
        if let Some(svc) = &existing {
            let pid = svc.query_status().ok().and_then(|s| s.process_id);
            stop_and_wait(svc);
            if let Some(pid) = pid {
                wait_exit(pid);
            }
        }
        let r = setup(&layout, sid, gui, &wintun, &cores);
        if let (Err(_), Some(svc)) = (&r, &existing) {
            // Don't leave the user without the service they had.
            let _ = svc.start::<&str>(&[]);
        }
        r?;

        let info = ServiceInfo {
            name: SERVICE_NAME.into(),
            display_name: "Prism TUN Service".into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: layout.service_exe.clone(),
            launch_arguments: vec!["run".into()],
            dependencies: vec![],
            account_name: None, // LocalSystem
            account_password: None,
        };
        let svc = match existing {
            Some(svc) => {
                svc.change_config(&info)?;
                svc
            }
            None => m.create_service(&info, access)?,
        };
        let _ = svc.set_description("Runs the Prism proxy core for TUN mode so Prism doesn't need administrator rights.");
        svc.start::<&str>(&[]).context("start service")?;
        for _ in 0..50 {
            if svc.query_status()?.current_state == ServiceState::Running {
                log(&layout, &format!("installed for {sid}"));
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        bail!("service did not start, see {}", layout.log_file().display())
    }

    /// Copies the binaries and writes the owner, folder permissions included.
    fn setup(layout: &Layout, sid: &str, gui: PathBuf, wintun: &Path, cores: &[(&str, PathBuf)]) -> Result<()> {
        // Binaries: admin-only folder.
        std::fs::create_dir_all(layout.cores_dir())?;
        copy_file(&std::env::current_exe()?, &layout.service_exe)?;
        copy_file(wintun, &layout.wintun())?;
        for (_, name) in CORES {
            if !cores.iter().any(|(f, _)| *f == name) {
                let _ = std::fs::remove_file(layout.cores_dir().join(name));
            }
        }
        for (file, from) in cores {
            copy_file(from, &layout.cores_dir().join(file))?;
        }

        // Data: SYSTEM and admins write, the owner may only read. Set on the
        // folder; everything inside inherits it.
        std::fs::create_dir_all(layout.run_dir())?;
        let status = std::process::Command::new("icacls")
            .arg(&layout.data)
            .args(["/inheritance:r", "/grant:r", "*S-1-5-18:(OI)(CI)F", "*S-1-5-32-544:(OI)(CI)F"])
            .arg(format!("*{sid}:(OI)(CI)RX"))
            .arg("/Q")
            .stdout(Stdio::null())
            .status()?;
        if !status.success() {
            bail!("icacls failed on {}", layout.data.display());
        }
        // Files from an earlier install go back to inheriting it.
        let _ = std::process::Command::new("icacls")
            .arg(layout.data.join("*"))
            .args(["/reset", "/T", "/C", "/Q"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        std::fs::write(layout.owner_file(), serde_json::to_vec_pretty(&Owner { user_sid: sid.into(), gui_path: gui })?)?;
        Ok(())
    }

    pub fn uninstall() -> Result<()> {
        let layout = Layout::system();
        let m = manager(ServiceManagerAccess::CONNECT)?;
        if let Ok(svc) = m.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE) {
            stop_and_wait(&svc);
            svc.delete()?;
        }
        // The running copy may be the one in program_dir; Windows removes a
        // deleted-but-open file once it closes.
        let _ = std::fs::remove_dir_all(&layout.data);
        let _ = std::fs::remove_dir_all(layout.cores_dir());
        let _ = std::fs::remove_file(layout.wintun());
        let _ = std::fs::remove_file(&layout.service_exe);
        let _ = std::fs::remove_dir(&layout.program);
        Ok(())
    }

    define_windows_service!(ffi_service_main, service_main);

    pub fn run_service() -> Result<()> {
        service_dispatcher::start(SERVICE_NAME, ffi_service_main).context("not started by the service manager")
    }

    fn service_main(_args: Vec<OsString>) {
        if let Err(e) = serve() {
            log(&Layout::system(), &format!("service failed: {e:#}"));
        }
    }

    fn serve() -> Result<()> {
        let layout = Layout::system();
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let handle = service_control_handler::register(SERVICE_NAME, move |ev| match ev {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = stop_tx.send(true);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        })?;
        let status = |state, accept| ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::from_secs(5),
            process_id: None,
        };
        handle.set_service_status(status(ServiceState::Running, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN))?;
        log(&layout, "service started");
        let rt = tokio::runtime::Runtime::new()?;
        let r = rt.block_on(server::serve(layout.clone(), stop_rx));
        handle.set_service_status(status(ServiceState::Stopped, ServiceControlAccept::empty()))?;
        log(&layout, "service stopped");
        r
    }
}
