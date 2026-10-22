mod core;
mod download;
mod store;
mod sysproxy;

use std::sync::Mutex;

use prism_core::config::Mode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};

use crate::core::CoreManager;
use crate::store::{CoreKind, ProfileMeta, Settings, Store};

pub struct AppState {
    store: Store,
    settings: Mutex<Settings>,
    core: CoreManager,
}

type Res<T> = Result<T, String>;

fn e(err: impl std::fmt::Display) -> String {
    format!("{err:#}")
}

impl AppState {
    fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    fn update(&self, f: impl FnOnce(&mut Settings)) -> Res<Settings> {
        let mut s = self.settings.lock().unwrap();
        f(&mut s);
        self.store.save_settings(&s).map_err(e)?;
        Ok(s.clone())
    }

    async fn proxy_port(&self) -> Option<u16> {
        self.core.running().await.map(|_| self.settings().mixed_port)
    }
}

#[derive(Serialize)]
struct Snapshot {
    settings: Settings,
    profiles: Vec<ProfileMeta>,
    running: Option<CoreKind>,
    system_proxy: bool,
    core_paths: Value,
    data_dir: String,
}

#[tauri::command]
async fn get_state(st: State<'_, AppState>) -> Res<Snapshot> {
    let s = st.settings();
    let (on, server) = sysproxy::get();
    let path = |k| core::core_binary(&st.store, &s, k);
    let paths = json!({
        "prism": { "path": path(CoreKind::Prism), "exists": path(CoreKind::Prism).exists() },
        "mihomo": { "path": path(CoreKind::Mihomo), "exists": path(CoreKind::Mihomo).exists() },
        "singbox": { "path": path(CoreKind::Singbox), "exists": path(CoreKind::Singbox).exists() },
    });
    Ok(Snapshot {
        system_proxy: on && server == format!("127.0.0.1:{}", s.mixed_port),
        settings: s,
        profiles: st.store.profiles(),
        running: st.core.running().await,
        core_paths: paths,
        data_dir: st.store.dir.display().to_string(),
    })
}

#[tauri::command]
async fn core_versions(st: State<'_, AppState>) -> Res<Value> {
    let s = st.settings();
    let mut out = serde_json::Map::new();
    for (k, kind) in [("prism", CoreKind::Prism), ("mihomo", CoreKind::Mihomo), ("singbox", CoreKind::Singbox)] {
        let v = download::core_version(kind, &core::core_binary(&st.store, &s, kind)).await;
        out.insert(k.into(), json!(v));
    }
    Ok(Value::Object(out))
}

#[tauri::command]
async fn save_settings(app: AppHandle, st: State<'_, AppState>, settings: Settings) -> Res<()> {
    let old = st.settings();
    let new = st.update(|s| {
        let selections = std::mem::take(&mut s.selections);
        *s = settings;
        s.selections = selections;
    })?;
    let restart = old.core != new.core
        || old.mixed_port != new.mixed_port
        || old.controller_port != new.controller_port
        || old.allow_lan != new.allow_lan
        || old.log_level != new.log_level
        || old.secret != new.secret;
    if restart && st.core.running().await.is_some() {
        start(&app, &st).await?;
    }
    if new.system_proxy {
        sysproxy::set(true, &format!("127.0.0.1:{}", new.mixed_port), &new.proxy_bypass).map_err(e)?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct ImportArgs {
    url: Option<String>,
    content: Option<String>,
    name: Option<String>,
}

async fn fetch_subscription(st: &AppState, url: &str) -> Res<(String, Option<store::Usage>, Option<String>)> {
    // Try directly first; fall back to the running proxy.
    let mut last = String::new();
    for proxy in [None, st.proxy_port().await].into_iter().collect::<std::collections::BTreeSet<_>>() {
        let client = download::client(proxy).map_err(e)?;
        match client.get(url).header("User-Agent", "clash.meta").send().await {
            Ok(r) if r.status().is_success() => {
                let usage = r
                    .headers()
                    .get("subscription-userinfo")
                    .and_then(|v| v.to_str().ok())
                    .map(store::parse_userinfo);
                let name = r
                    .headers()
                    .get("content-disposition")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.split("filename=").nth(1))
                    .map(|n| n.trim_matches('"').to_string());
                return Ok((r.text().await.map_err(e)?, usage, name));
            }
            Ok(r) => last = format!("HTTP {}", r.status()),
            Err(err) => last = e(err),
        }
    }
    Err(format!("下载订阅失败: {last}"))
}

#[tauri::command]
async fn import_profile(app: AppHandle, st: State<'_, AppState>, args: ImportArgs) -> Res<ProfileMeta> {
    let url = args.url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
    let (content, usage, remote_name) = match (&url, args.content) {
        (Some(u), _) => fetch_subscription(&st, u).await?,
        (None, Some(c)) => (c, None, None),
        (None, None) => return Err("请输入订阅链接或配置内容".into()),
    };
    let name = args
        .name
        .filter(|n| !n.trim().is_empty())
        .or(remote_name)
        .or_else(|| url.as_ref().and_then(|u| reqwest::Url::parse(u).ok()).and_then(|u| u.host_str().map(String::from)))
        .unwrap_or_else(|| "本地配置".into());
    let meta = st.store.put(None, &name, url, &content, usage).map_err(e)?;
    if st.settings().active_profile.is_none() {
        activate_profile(app, st, meta.id.clone()).await?;
    }
    Ok(meta)
}

#[tauri::command]
async fn update_profile(app: AppHandle, st: State<'_, AppState>, id: String) -> Res<ProfileMeta> {
    let meta = st.store.profiles().into_iter().find(|p| p.id == id).ok_or("订阅不存在")?;
    let url = meta.url.clone().ok_or("本地配置无需更新")?;
    let (content, usage, _) = fetch_subscription(&st, &url).await?;
    let meta = st.store.put(Some(&id), &meta.name, Some(url), &content, usage).map_err(e)?;
    if st.settings().active_profile.as_deref() == Some(id.as_str()) && st.core.running().await.is_some() {
        start(&app, &st).await?;
    }
    Ok(meta)
}

#[tauri::command]
async fn save_profile_content(app: AppHandle, st: State<'_, AppState>, id: String, content: String) -> Res<ProfileMeta> {
    let meta = st.store.profiles().into_iter().find(|p| p.id == id).ok_or("订阅不存在")?;
    let meta = st.store.put(Some(&id), &meta.name, meta.url, &content, meta.usage).map_err(e)?;
    if st.settings().active_profile.as_deref() == Some(id.as_str()) && st.core.running().await.is_some() {
        start(&app, &st).await?;
    }
    Ok(meta)
}

#[tauri::command]
fn rename_profile(st: State<'_, AppState>, id: String, name: String) -> Res<()> {
    st.store.rename(&id, &name).map_err(e)
}

#[tauri::command]
fn read_profile(st: State<'_, AppState>, id: String) -> Res<String> {
    st.store.read_content(&id).map_err(e)
}

#[tauri::command]
async fn delete_profile(st: State<'_, AppState>, id: String) -> Res<()> {
    if st.settings().active_profile.as_deref() == Some(id.as_str()) {
        if st.core.running().await.is_some() {
            return Err("不能删除正在使用的订阅，请先停止内核或切换订阅".into());
        }
        st.update(|s| s.active_profile = None)?;
    }
    st.update(|s| {
        s.selections.remove(&id);
    })?;
    st.store.delete(&id).map_err(e)
}

#[tauri::command]
async fn activate_profile(app: AppHandle, st: State<'_, AppState>, id: String) -> Res<()> {
    st.update(|s| s.active_profile = Some(id))?;
    if st.core.running().await.is_some() {
        start(&app, &st).await?;
    }
    Ok(())
}

async fn start(app: &AppHandle, st: &AppState) -> Res<()> {
    let s = st.settings();
    // prism-core needs geodata for GEOSITE/GEOIP rules; fetch it once.
    let rt = st.store.runtime_dir();
    if s.core == CoreKind::Prism && !(rt.join("geosite.dat").exists() && rt.join("geoip.dat").exists()) {
        st.core.push_log(app, "[prism] 正在下载 geosite.dat / geoip.dat ...".into());
        if let Err(err) = download::geodata(&rt, None).await {
            st.core.push_log(app, format!("[prism] geodata 下载失败（GEOSITE/GEOIP 规则将被跳过）: {err:#}"));
        }
    }
    st.core.start(app, &st.store, &s).await.map_err(e)?;
    // Re-apply the user's node choices for this profile.
    if let Some(sel) = s.active_profile.as_ref().and_then(|id| s.selections.get(id)) {
        for (group, name) in sel {
            let path = format!("/proxies/{}", encode(group));
            let _ = core::api(&s, "PUT", &path, Some(json!({ "name": name }))).await;
        }
    }
    let _ = app.emit("core-changed", ());
    refresh_tray(app);
    Ok(())
}

fn encode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

#[tauri::command]
async fn start_core(app: AppHandle, st: State<'_, AppState>) -> Res<()> {
    start(&app, &st).await
}

#[tauri::command]
async fn stop_core(app: AppHandle, st: State<'_, AppState>) -> Res<()> {
    st.core.stop().await;
    st.core.push_log(&app, "[prism] 内核已停止".into());
    if st.settings().system_proxy {
        set_system_proxy(app.clone(), st, false).await?;
    }
    let _ = app.emit("core-changed", ());
    refresh_tray(&app);
    Ok(())
}

#[tauri::command]
async fn core_api(st: State<'_, AppState>, method: String, path: String, body: Option<Value>) -> Res<Value> {
    core::api(&st.settings(), &method, &path, body).await.map_err(e)
}

#[tauri::command]
async fn select_proxy(st: State<'_, AppState>, group: String, name: String) -> Res<()> {
    let s = st.settings();
    core::api(&s, "PUT", &format!("/proxies/{}", encode(&group)), Some(json!({ "name": name.clone() })))
        .await
        .map_err(e)?;
    // Close existing connections so the new node takes effect immediately.
    let _ = core::api(&s, "DELETE", "/connections", None).await;
    if let Some(id) = s.active_profile.clone() {
        st.update(|s| {
            s.selections.entry(id).or_default().insert(group, name);
        })?;
    }
    Ok(())
}

#[tauri::command]
async fn set_mode(app: AppHandle, st: State<'_, AppState>, mode: String) -> Res<()> {
    let m = Mode::parse(&mode).ok_or("unknown mode")?;
    st.update(|s| s.mode = m)?;
    if st.core.running().await.is_some() {
        let s = st.settings();
        // sing-box names its modes Rule/Global/Direct.
        let name = if s.core == CoreKind::Singbox {
            let mut c = mode.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        } else {
            mode
        };
        core::api(&s, "PATCH", "/configs", Some(json!({ "mode": name }))).await.map_err(e)?;
    }
    refresh_tray(&app);
    Ok(())
}

#[tauri::command]
async fn set_system_proxy(app: AppHandle, st: State<'_, AppState>, enable: bool) -> Res<()> {
    let s = st.update(|s| s.system_proxy = enable)?;
    sysproxy::set(enable, &format!("127.0.0.1:{}", s.mixed_port), &s.proxy_bypass).map_err(e)?;
    let _ = app.emit("core-changed", ());
    refresh_tray(&app);
    Ok(())
}

#[tauri::command]
fn get_logs(st: State<'_, AppState>) -> Vec<String> {
    st.core.logs.lock().unwrap().iter().cloned().collect()
}

#[tauri::command]
fn clear_logs(st: State<'_, AppState>) {
    st.core.logs.lock().unwrap().clear();
}

#[tauri::command]
async fn download_core(st: State<'_, AppState>, kind: CoreKind) -> Res<String> {
    let port = st.proxy_port().await;
    download::core(kind, &st.store.cores_dir(), port).await.map_err(e)
}

#[tauri::command]
async fn update_geodata(st: State<'_, AppState>) -> Res<()> {
    let port = st.proxy_port().await;
    download::geodata(&st.store.runtime_dir(), port).await.map_err(e)
}

#[tauri::command]
fn preview_config(st: State<'_, AppState>) -> Res<String> {
    let path = core::write_runtime_config(&st.store, &st.settings()).map_err(e)?;
    std::fs::read_to_string(path).map_err(e)
}

#[tauri::command]
fn open_data_dir(st: State<'_, AppState>) -> Res<()> {
    std::process::Command::new("explorer").arg(&st.store.dir).spawn().map_err(e)?;
    Ok(())
}

// ------------------------------------------------------------------ tray

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn build_tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let st = app.state::<AppState>();
    let s = st.settings();
    let show = MenuItem::with_id(app, "show", "显示主界面", true, None::<&str>)?;
    let sys = CheckMenuItem::with_id(app, "sysproxy", "系统代理", true, s.system_proxy, None::<&str>)?;
    let rule = CheckMenuItem::with_id(app, "mode_rule", "规则模式", true, s.mode == Mode::Rule, None::<&str>)?;
    let global = CheckMenuItem::with_id(app, "mode_global", "全局模式", true, s.mode == Mode::Global, None::<&str>)?;
    let direct = CheckMenuItem::with_id(app, "mode_direct", "直连模式", true, s.mode == Mode::Direct, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    Menu::with_items(
        app,
        &[&show, &PredefinedMenuItem::separator(app)?, &sys, &rule, &global, &direct, &PredefinedMenuItem::separator(app)?, &quit],
    )
}

fn refresh_tray(app: &AppHandle) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id("main"), build_tray_menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn on_menu(app: &AppHandle, id: &str) {
    let app = app.clone();
    let id = id.to_string();
    tauri::async_runtime::spawn(async move {
        let st = app.state::<AppState>();
        let r = match id.as_str() {
            "show" => Ok(show_main(&app)),
            "sysproxy" => {
                let on = !st.settings().system_proxy;
                set_system_proxy(app.clone(), st.clone(), on).await
            }
            "mode_rule" | "mode_global" | "mode_direct" => {
                set_mode(app.clone(), st.clone(), id.trim_start_matches("mode_").to_string()).await
            }
            "quit" => {
                app.exit(0);
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(err) = r {
            st.core.push_log(&app, format!("[prism] {err}"));
        }
        let _ = app.emit("core-changed", ());
        refresh_tray(&app);
    });
}

pub fn run() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let store = Store::new(dir)?;
            let settings = store.settings();
            app.manage(AppState { store, settings: Mutex::new(settings), core: CoreManager::default() });

            let menu = build_tray_menu(app.handle())?;
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Prism")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, ev| on_menu(app, ev.id().as_ref()))
                .on_tray_icon_event(|tray, ev| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;

            // Auto-start the core when a profile is configured.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let st = handle.state::<AppState>();
                let s = st.settings();
                if s.active_profile.is_some() {
                    match start(&handle, &st).await {
                        Ok(()) if s.system_proxy => {
                            let _ = sysproxy::set(true, &format!("127.0.0.1:{}", s.mixed_port), &s.proxy_bypass);
                        }
                        Ok(()) => {}
                        Err(err) => st.core.push_log(&handle, format!("[prism] 自动启动失败: {err}")),
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|w, ev| {
            // Closing the window keeps the app in the tray.
            if let WindowEvent::CloseRequested { api, .. } = ev {
                api.prevent_close();
                let _ = w.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            core_versions,
            save_settings,
            import_profile,
            update_profile,
            save_profile_content,
            rename_profile,
            read_profile,
            delete_profile,
            activate_profile,
            start_core,
            stop_core,
            core_api,
            select_proxy,
            set_mode,
            set_system_proxy,
            get_logs,
            clear_logs,
            download_core,
            update_geodata,
            preview_config,
            open_data_dir,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, ev| {
        if let RunEvent::Exit = ev {
            let st = app.state::<AppState>();
            if st.settings().system_proxy {
                let _ = sysproxy::set(false, "", "");
            }
            tauri::async_runtime::block_on(st.core.stop());
        }
    });
}
