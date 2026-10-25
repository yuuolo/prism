mod autostart;
mod backup;
mod core;
mod download;
mod patches;
mod elevate;
mod links;
mod store;
mod sysproxy;
mod traffic;
mod tunsvc;
mod unlock;
mod updater;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use prism_core::config::Mode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};

use crate::core::CoreManager;
use crate::store::{CoreKind, ProfileMeta, Settings, Store};

pub struct AppState {
    store: Store,
    settings: Mutex<Settings>,
    core: CoreManager,
    /// Release found by the last update check.
    update: Mutex<Option<tauri_plugin_updater::Update>>,
    /// Import links waiting for the window to confirm them.
    imports: Mutex<Vec<PendingLink>>,
    /// One tray menu rebuild at a time, so an older one can't land last.
    tray_lock: tokio::sync::Mutex<()>,
    traffic: traffic::Traffic,
    /// Registered global hotkey id -> action.
    hotkeys: Mutex<HashMap<u32, String>>,
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
    elevated: bool,
    core_paths: Value,
    data_dir: String,
    version: String,
    /// Windows starts Prism at logon.
    autostart: bool,
    /// clash:// links open Prism.
    clash_links: bool,
    /// File name of the program that opens clash:// links now.
    clash_handler: Option<String>,
    /// TUN service: missing / stopped / running.
    tun_service: tunsvc::State,
}

#[tauri::command]
async fn get_state(app: AppHandle, st: State<'_, AppState>) -> Res<Snapshot> {
    let mut s = st.settings();
    if !s.webdav.password.is_empty() {
        // The UI only needs to know a password is stored.
        s.webdav.password = "saved".into();
    }
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
        elevated: elevate::is_elevated(),
        core_paths: paths,
        data_dir: st.store.dir.display().to_string(),
        version: app.package_info().version.to_string(),
        autostart: autostart::enabled(),
        clash_links: links::owned("clash"),
        clash_handler: links::handler("clash"),
        tun_service: tunsvc::state(),
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
        // Fields with their own commands keep their live values.
        let keep = s.clone();
        *s = settings;
        s.selections = keep.selections;
        s.tun = keep.tun;
        s.system_proxy = keep.system_proxy;
        s.mode = keep.mode;
        s.active_profile = keep.active_profile;
        s.app_rules = keep.app_rules;
        s.webdav = keep.webdav;
    })?;
    let restart = old.core != new.core
        || old.mixed_port != new.mixed_port
        || old.controller_port != new.controller_port
        || old.allow_lan != new.allow_lan
        || old.log_level != new.log_level
        || old.secret != new.secret
        || old.unified_delay != new.unified_delay;
    if restart && st.core.running().await.is_some() {
        start(&app, &st).await?;
    }
    if new.system_proxy {
        sysproxy::set(true, &format!("127.0.0.1:{}", new.mixed_port), &new.proxy_bypass).map_err(e)?;
    }
    apply_theme(&app, &new.theme);
    refresh_tray(&app);
    let errors = apply_hotkeys(&app, &new);
    if !errors.is_empty() {
        return Err(format!("设置已保存，但以下快捷键无法注册（格式不对或已被其他程序占用）：{}", errors.join("；")));
    }
    Ok(())
}

/// Window frame follows the chosen theme (None = the system's).
fn apply_theme(app: &AppHandle, theme: &str) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_theme(match theme {
            "light" => Some(tauri::Theme::Light),
            "dark" => Some(tauri::Theme::Dark),
            _ => None,
        });
    }
}

/// Global hotkey actions; the rest are tray menu ids.
const HOTKEY_ACTIONS: [&str; 6] = ["toggle_window", "sysproxy", "tun", "mode_rule", "mode_global", "mode_direct"];

/// (Re)registers the hotkeys from settings; returns the ones that failed.
fn apply_hotkeys(app: &AppHandle, s: &Settings) -> Vec<String> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let mut map = HashMap::new();
    let mut failed = Vec::new();
    for (action, accel) in &s.hotkeys {
        let accel = accel.trim();
        if accel.is_empty() || !HOTKEY_ACTIONS.contains(&action.as_str()) {
            continue;
        }
        match accel.parse::<Shortcut>() {
            Ok(sc) => match gs.register(sc) {
                Ok(()) => {
                    map.insert(sc.id(), action.clone());
                }
                Err(err) => failed.push(format!("{accel}（{err}）")),
            },
            Err(err) => failed.push(format!("{accel}（{err}）")),
        }
    }
    if let Some(st) = app.try_state::<AppState>() {
        *st.hotkeys.lock().unwrap() = map;
    }
    failed
}

fn on_hotkey(app: &AppHandle, action: &str) {
    if action == "toggle_window" {
        let visible = app.get_webview_window("main").map(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false));
        if visible == Some(true) {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.hide();
            }
        } else {
            show_main(app);
        }
        return;
    }
    on_menu(app, action);
}

/// Saves the WebDAV target; `password` None keeps the stored one.
#[tauri::command]
fn webdav_save(st: State<'_, AppState>, url: String, username: String, password: Option<String>) -> Res<()> {
    let enc = match password {
        Some(p) => Some(backup::protect(&p).map_err(e)?),
        None => None,
    };
    st.update(|s| {
        s.webdav.url = url.trim().to_string();
        s.webdav.username = username.trim().to_string();
        if let Some(enc) = enc {
            s.webdav.password = enc;
        }
    })?;
    Ok(())
}

fn dav(st: &AppState) -> Res<backup::Dav> {
    backup::Dav::new(&st.settings().webdav).map_err(e)
}

#[tauri::command]
async fn webdav_test(st: State<'_, AppState>) -> Res<()> {
    dav(&st)?.ensure_dir().await.map_err(e)
}

/// Uploads a backup; returns its file name.
#[tauri::command]
async fn backup_upload(st: State<'_, AppState>) -> Res<String> {
    let data = backup::create(&st.store, &st.settings()).map_err(e)?;
    let name = backup::file_name();
    dav(&st)?.upload(&name, data).await.map_err(e)?;
    Ok(name)
}

#[tauri::command]
async fn backup_list(st: State<'_, AppState>) -> Res<Vec<backup::RemoteBackup>> {
    dav(&st)?.list().await.map_err(e)
}

#[tauri::command]
async fn backup_delete(st: State<'_, AppState>, name: String) -> Res<()> {
    dav(&st)?.delete(&name).await.map_err(e)
}

#[tauri::command]
async fn backup_restore(app: AppHandle, st: State<'_, AppState>, name: String) -> Res<()> {
    let data = dav(&st)?.download(&name).await.map_err(e)?;
    restore_backup(&app, &st, &data).await
}

/// Restores a backup file the user picked (base64).
#[tauri::command]
async fn backup_import(app: AppHandle, st: State<'_, AppState>, data: String) -> Res<()> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(e)?;
    restore_backup(&app, &st, &bytes).await
}

/// Writes a backup to the Downloads folder and shows it in Explorer.
#[tauri::command]
fn backup_export(app: AppHandle, st: State<'_, AppState>) -> Res<String> {
    let data = backup::create(&st.store, &st.settings()).map_err(e)?;
    let dir = app.path().download_dir().map_err(e)?;
    let path = dir.join(backup::file_name());
    std::fs::write(&path, data).map_err(e)?;
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    Ok(path.display().to_string())
}

async fn restore_backup(app: &AppHandle, st: &AppState, data: &[u8]) -> Res<()> {
    let s = {
        let current = st.settings();
        backup::restore(&st.store, &current, data).map_err(e)?
    };
    *st.settings.lock().unwrap() = s.clone();
    note(app, st, "已从备份恢复设置和订阅".into());
    apply_theme(app, &s.theme);
    let _ = apply_hotkeys(app, &s);
    if st.core.running().await.is_some() {
        if s.active_profile.is_some() {
            start(app, st).await?;
        } else {
            st.core.stop().await;
        }
    }
    let _ = app.emit("core-changed", ());
    refresh_tray(app);
    Ok(())
}

/// Streaming / AI availability through the running core.
#[tauri::command]
async fn unlock_check(st: State<'_, AppState>) -> Res<Vec<unlock::Check>> {
    let port = st.proxy_port().await.ok_or("请先启动内核")?;
    unlock::run_all(Some(port)).await.map_err(e)
}

#[tauri::command]
fn get_patches(st: State<'_, AppState>) -> patches::Patches {
    patches::load(&st.store.dir)
}

/// Saves 覆写 after trying it on the active profile, then applies it.
#[tauri::command]
async fn save_patches(app: AppHandle, st: State<'_, AppState>, patches: patches::Patches) -> Res<()> {
    let s = st.settings();
    if let Some(id) = &s.active_profile {
        let raw = st.store.read_content(id).map_err(e)?;
        let name = st.store.profiles().into_iter().find(|p| &p.id == id).map(|p| p.name).unwrap_or_default();
        let out = patches::apply(&patches, &raw, &name).map_err(e)?;
        prism_core::config::parse_any(&out).map_err(|err| format!("覆写后的配置无法使用：{err:#}"))?;
    }
    patches::save(&st.store.dir, &patches).map_err(e)?;
    reload_core(&app, &st).await
}

#[tauri::command]
fn traffic_history(st: State<'_, AppState>) -> Vec<traffic::Day> {
    st.traffic.history()
}

/// Samples the core's traffic counters into daily totals.
async fn traffic_sampler(app: AppHandle) {
    let mut tick = 0u32;
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let st = app.state::<AppState>();
        let totals = match st.core.running().await {
            Some(_) => core::api(&st.settings(), "GET", "/connections", None).await.ok(),
            None => None,
        };
        match totals {
            Some(v) => {
                let up = v["uploadTotal"].as_u64().unwrap_or(0);
                let down = v["downloadTotal"].as_u64().unwrap_or(0);
                st.traffic.record(&chrono::Local::now().format("%Y-%m-%d").to_string(), up, down);
            }
            None => st.traffic.reset_baseline(),
        }
        tick += 1;
        if tick % 12 == 0 {
            st.traffic.save();
        }
    }
}

#[derive(Deserialize)]
struct ImportArgs {
    url: Option<String>,
    content: Option<String>,
    name: Option<String>,
}

#[derive(Default)]
struct Subscription {
    content: String,
    usage: Option<store::Usage>,
    /// From Content-Disposition.
    name: Option<String>,
    /// Minutes, from the provider's `profile-update-interval` header.
    interval: Option<u32>,
}

/// File name from a Content-Disposition header, preferring the RFC 5987
/// `filename*=UTF-8''…` form that carries non-ASCII names.
fn disposition_name(h: &str) -> Option<String> {
    let mut plain = None;
    for part in h.split(';') {
        let Some((k, v)) = part.split_once('=') else { continue };
        let v = v.trim();
        match k.trim().to_ascii_lowercase().as_str() {
            "filename*" => {
                let value = v.rsplit('\'').next().unwrap_or(v);
                let name = percent_encoding::percent_decode_str(value).decode_utf8_lossy().trim().to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
            "filename" => plain = Some(v.trim_matches('"').trim().to_string()),
            _ => {}
        }
    }
    plain.filter(|n| !n.is_empty())
}

/// `profile-update-interval` is in hours; profiles store minutes.
fn interval_minutes(hours: &str) -> Option<u32> {
    let h = hours.trim().parse::<f64>().ok().filter(|h| h.is_finite() && *h >= 0.0)?;
    Some((h * 60.0).round().min(u32::MAX as f64) as u32)
}

async fn fetch_subscription(st: &AppState, url: &str) -> Res<Subscription> {
    // Try directly first; fall back to the running proxy.
    let mut last = String::new();
    for proxy in [None, st.proxy_port().await].into_iter().collect::<std::collections::BTreeSet<_>>() {
        let client = download::client(proxy).map_err(e)?;
        match client.get(url).header("User-Agent", "clash.meta").send().await {
            Ok(r) if r.status().is_success() => {
                let header = |k: &str| r.headers().get(k).and_then(|v| v.to_str().ok()).map(String::from);
                let usage = header("subscription-userinfo").map(|v| store::parse_userinfo(&v));
                let name = header("content-disposition").and_then(|v| disposition_name(&v));
                let interval = header("profile-update-interval").and_then(|v| interval_minutes(&v));
                return Ok(Subscription { content: r.text().await.map_err(e)?, usage, name, interval });
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
    let sub = match (&url, args.content) {
        (Some(u), _) => fetch_subscription(&st, u).await?,
        (None, Some(c)) => Subscription { content: c, ..Default::default() },
        (None, None) => return Err("请输入订阅链接或配置内容".into()),
    };
    let name = args
        .name
        .filter(|n| !n.trim().is_empty())
        .or(sub.name)
        .or_else(|| url.as_ref().and_then(|u| reqwest::Url::parse(u).ok()).and_then(|u| u.host_str().map(String::from)))
        .unwrap_or_else(|| "本地配置".into());
    let meta = st.store.put(None, &name, url, &sub.content, sub.usage, sub.interval).map_err(e)?;
    if st.settings().active_profile.is_none() {
        activate_profile(app, st, meta.id.clone()).await?;
    }
    Ok(meta)
}

#[tauri::command]
async fn update_profile(app: AppHandle, st: State<'_, AppState>, id: String) -> Res<ProfileMeta> {
    refresh_profile(&app, &st, &id).await
}

/// Downloads a remote profile again and applies it if it's in use.
async fn refresh_profile(app: &AppHandle, st: &AppState, id: &str) -> Res<ProfileMeta> {
    let meta = st.store.profiles().into_iter().find(|p| p.id == id).ok_or("订阅不存在")?;
    let url = meta.url.clone().ok_or("本地配置无需更新")?;
    let sub = fetch_subscription(st, &url).await?;
    let meta = st.store.put(Some(id), &meta.name, Some(url), &sub.content, sub.usage, None).map_err(e)?;
    if st.settings().active_profile.as_deref() == Some(id) {
        reload_core(app, st).await?;
    }
    Ok(meta)
}

#[tauri::command]
async fn save_profile_content(app: AppHandle, st: State<'_, AppState>, id: String, content: String) -> Res<ProfileMeta> {
    let meta = st.store.profiles().into_iter().find(|p| p.id == id).ok_or("订阅不存在")?;
    let meta = st.store.put(Some(&id), &meta.name, meta.url, &content, meta.usage, None).map_err(e)?;
    if st.settings().active_profile.as_deref() == Some(id.as_str()) {
        reload_core(&app, &st).await?;
    }
    Ok(meta)
}

#[tauri::command]
fn set_profile_interval(st: State<'_, AppState>, id: String, minutes: u32) -> Res<()> {
    st.store.set_interval(&id, minutes).map_err(e)
}

/// How often the scheduler looks for subscriptions that are due.
const SCHEDULE_TICK: Duration = Duration::from_secs(60);
/// Seconds before a failed automatic update is tried again.
const RETRY_SECS: i64 = 30 * 60;

/// Updates remote profiles once their update interval has passed.
async fn subscription_scheduler(app: AppHandle) {
    let mut failed: HashMap<String, i64> = HashMap::new();
    loop {
        tokio::time::sleep(SCHEDULE_TICK).await;
        let st = app.state::<AppState>();
        for p in st.store.profiles() {
            if p.url.is_none() || p.interval == 0 {
                continue;
            }
            let now = chrono::Utc::now().timestamp();
            let every = i64::from(p.interval) * 60;
            let retry_at = failed.get(&p.id).map_or(0, |t| t + RETRY_SECS.min(every));
            if now < p.updated_at + every || now < retry_at {
                continue;
            }
            match refresh_profile(&app, &st, &p.id).await {
                Ok(m) => {
                    failed.remove(&p.id);
                    note(&app, &st, format!("订阅「{}」已自动更新（{} 个节点）", m.name, m.nodes));
                }
                Err(err) => {
                    failed.insert(p.id.clone(), now);
                    note(&app, &st, format!("订阅「{}」自动更新失败: {err}", p.name));
                }
            }
        }
    }
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

/// Error prefix the UI recognises to offer the TUN service or "restart as
/// administrator".
const NEED_ADMIN: &str = "NEED_ADMIN: TUN 模式需要管理员权限";

/// TUN can run: Prism is elevated or the TUN service is up.
fn tun_allowed() -> bool {
    elevate::is_elevated() || tunsvc::state() == tunsvc::State::Running
}

async fn start(app: &AppHandle, st: &AppState) -> Res<()> {
    let s = st.settings();
    if s.tun && !tun_allowed() {
        return Err(NEED_ADMIN.into());
    }
    if s.tun && s.core == CoreKind::Prism && !st.store.runtime_dir().join("wintun.dll").exists() {
        st.core.push_log(app, "[prism] 正在下载 wintun.dll ...".into());
        download::wintun(&st.store.runtime_dir(), None).await.map_err(|err| format!("下载 wintun.dll 失败: {err:#}"))?;
    }
    // prism-core needs geodata for GEOSITE/GEOIP rules; fetch it once.
    let rt = st.store.runtime_dir();
    if s.core == CoreKind::Prism && !(rt.join("geosite.dat").exists() && rt.join("geoip.dat").exists()) {
        st.core.push_log(app, "[prism] 正在下载 geosite.dat / geoip.dat ...".into());
        if let Err(err) = download::geodata(&rt, None).await {
            st.core.push_log(app, format!("[prism] geodata 下载失败（GEOSITE/GEOIP 规则将被跳过）: {err:#}"));
        }
    }
    if s.core == CoreKind::Singbox {
        // Errors (e.g. no profile yet) surface from core.start below.
        let _ = ensure_rule_sets(app, st, false).await;
    }
    st.core.start(app, &st.store, &s).await.map_err(e)?;
    apply_selections(&s).await;
    let _ = app.emit("core-changed", ());
    refresh_tray(app);
    Ok(())
}

/// Re-applies the user's node choices for the active profile.
async fn apply_selections(s: &Settings) {
    if let Some(sel) = s.active_profile.as_ref().and_then(|id| s.selections.get(id)) {
        for (group, name) in sel {
            let path = format!("/proxies/{}", encode(group));
            let _ = core::api(s, "PUT", &path, Some(json!({ "name": name }))).await;
        }
    }
}

/// Applies a changed active profile to a running core. Prism and mihomo
/// reload in place, keeping listeners and open connections; sing-box has no
/// reload API and restarts. A config the core rejects leaves it running the
/// old one.
async fn reload_core(app: &AppHandle, st: &AppState) -> Res<()> {
    let s = st.settings();
    match st.core.running().await {
        None => return Ok(()),
        Some(kind) if kind != s.core || kind == CoreKind::Singbox => return start(app, st).await,
        Some(_) => {}
    }
    let cfg = core::write_runtime_config(&st.store, &s).map_err(e)?;
    for w in cfg.warnings {
        st.core.push_log(app, format!("[prism] {w}"));
    }
    core::api(&s, "PUT", "/configs", Some(json!({ "path": cfg.path.to_string_lossy() })))
        .await
        .map_err(|err| format!("内核未能加载新配置，仍在使用旧配置: {err:#}"))?;
    apply_selections(&s).await;
    note(app, st, "配置已热重载".into());
    let _ = app.emit("core-changed", ());
    refresh_tray(app);
    Ok(())
}

const RULE_SET_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

/// Downloads the sing-box rule-sets the active profile needs. Missing files
/// are fetched now; stale ones are refreshed in the background (or now when
/// `force`). Returns how many are still unavailable.
async fn ensure_rule_sets(app: &AppHandle, st: &AppState, force: bool) -> Res<usize> {
    let sets = core::singbox_rule_sets(&st.store, &st.settings()).map_err(e)?;
    let dir = st.store.rule_set_dir();
    let port = st.proxy_port().await;
    let mut now = Vec::new();
    for r in sets {
        let path = dir.join(&r.file);
        let age = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
        match age {
            None => now.push((r, path)),
            Some(_) if force => now.push((r, path)),
            Some(a) if a > RULE_SET_MAX_AGE => {
                tauri::async_runtime::spawn(async move {
                    let _ = download::rule_set(&r.url, &path, port).await;
                });
            }
            Some(_) => {}
        }
    }
    if now.is_empty() {
        return Ok(0);
    }
    let tags: Vec<&str> = now.iter().map(|(r, _)| r.tag.as_str()).collect();
    st.core.push_log(app, format!("[prism] 正在下载规则集: {}", tags.join(", ")));
    let results = futures::future::join_all(now.iter().map(|(r, path)| download::rule_set(&r.url, path, port))).await;
    let mut failed = 0;
    for ((r, path), res) in now.iter().zip(results) {
        if let Err(err) = res {
            // A stale copy still works; only a missing file is a failure.
            if !path.exists() {
                failed += 1;
            }
            st.core.push_log(app, format!("[prism] 规则集 {} 下载失败: {err:#}", r.tag));
        }
    }
    Ok(failed)
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
async fn select_proxy(app: AppHandle, st: State<'_, AppState>, group: String, name: String) -> Res<()> {
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
    refresh_tray(&app);
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
async fn set_tun(app: AppHandle, st: State<'_, AppState>, enable: bool) -> Res<()> {
    if enable && !tun_allowed() {
        return Err(NEED_ADMIN.into());
    }
    st.update(|s| s.tun = enable)?;
    if st.core.running().await.is_some() {
        if let Err(err) = start(&app, &st).await {
            st.update(|s| s.tun = false)?;
            return Err(err);
        }
    }
    refresh_tray(&app);
    Ok(())
}

#[tauri::command]
fn restart_as_admin(app: AppHandle) -> Res<()> {
    if !elevate::relaunch_as_admin() {
        return Err("已取消".into());
    }
    app.exit(0);
    Ok(())
}

/// Logs to the app log (and stderr, visible in dev).
fn note(app: &AppHandle, st: &AppState, msg: String) {
    eprintln!("[prism] {msg}");
    st.core.push_log(app, format!("[prism] {msg}"));
}

#[tauri::command]
async fn check_update(app: AppHandle, st: State<'_, AppState>) -> Res<Option<updater::UpdateInfo>> {
    let found = updater::find(&app, st.proxy_port().await).await.map_err(|err| format!("检查更新失败: {err:#}"))?;
    let info = found.as_ref().map(updater::info);
    *st.update.lock().unwrap() = found;
    Ok(info)
}

/// Downloads + verifies the release, stops the core (the installer replaces
/// prism-core.exe) and runs the installer, which restarts Prism.
#[tauri::command]
async fn install_update(app: AppHandle, st: State<'_, AppState>) -> Res<()> {
    let pending = st.update.lock().unwrap().clone();
    let update = match pending {
        Some(u) => u,
        None => updater::find(&app, st.proxy_port().await).await.map_err(e)?.ok_or("已是最新版本")?,
    };
    note(&app, &st, format!("正在下载 v{} ...", update.version));
    let bytes = updater::download(&app, &update).await.map_err(e)?;
    note(&app, &st, format!("v{} 下载完成，签名校验通过，开始安装", update.version));
    let was_running = st.core.running().await.is_some();
    st.core.stop().await;
    if st.settings().system_proxy {
        // The setting stays on, so the new version turns it back on.
        let _ = sysproxy::set(false, "", "");
    }
    if let Err(err) = updater::install(&update, &bytes) {
        if was_running {
            let _ = start(&app, &st).await;
        }
        return Err(e(err));
    }
    Ok(())
}

#[tauri::command]
async fn list_processes() -> Res<Vec<prism_core::process::ProcessInfo>> {
    tauri::async_runtime::spawn_blocking(prism_core::process::list).await.map_err(e)
}

#[tauri::command]
fn profile_outbounds(st: State<'_, AppState>) -> Res<Value> {
    let (groups, proxies) = core::profile_outbounds(&st.store, &st.settings()).map_err(e)?;
    Ok(json!({ "groups": groups, "proxies": proxies }))
}

/// Saves the app rules and restarts a running core so they take effect.
#[tauri::command]
async fn set_app_rules(app: AppHandle, st: State<'_, AppState>, rules: Vec<store::AppRule>) -> Res<()> {
    st.update(|s| s.app_rules = rules)?;
    if st.core.running().await.is_some() {
        start(&app, &st).await?;
    }
    Ok(())
}

/// Installs the TUN service, or refreshes its copies of the cores. Shows a
/// UAC prompt. A running TUN core is restarted inside the service.
#[tauri::command]
async fn install_tun_service(app: AppHandle, st: State<'_, AppState>) -> Res<()> {
    tunsvc::install(&st.store, &st.settings()).await.map_err(e)?;
    note(&app, &st, "TUN 服务已安装 / 更新".into());
    if st.settings().tun && st.core.running().await.is_some() {
        start(&app, &st).await?;
    }
    Ok(())
}

#[tauri::command]
async fn uninstall_tun_service(app: AppHandle, st: State<'_, AppState>) -> Res<()> {
    let was_running = st.core.running().await.is_some();
    if was_running && core::use_service(&st.settings()) {
        st.core.stop().await;
    }
    tunsvc::uninstall().await.map_err(e)?;
    note(&app, &st, "TUN 服务已卸载".into());
    if st.settings().tun && !tun_allowed() {
        st.update(|s| s.tun = false)?;
    }
    if was_running && st.core.running().await.is_none() {
        start(&app, &st).await?;
    }
    let _ = app.emit("core-changed", ());
    Ok(())
}

#[tauri::command]
fn set_autostart(enable: bool) -> Res<()> {
    autostart::set(enable).map_err(e)
}

/// Makes clash:// links (subscription panels' "import to Clash") open Prism,
/// or gives them back to the program that had them.
#[tauri::command]
fn set_clash_links(enable: bool) -> Res<()> {
    if enable { links::take_over("clash") } else { links::release("clash") }.map_err(e)
}

/// A link waiting for the window: an import to confirm, or why the link
/// can't be used.
#[derive(Clone, Serialize)]
struct PendingLink {
    request: Option<links::ImportRequest>,
    error: Option<String>,
}

/// Queues a `prism://` / `clash://` link and shows the window to confirm it.
fn handle_link(app: &AppHandle, link: &str) {
    let Some(st) = app.try_state::<AppState>() else { return };
    let pending = match links::parse(link) {
        Ok(r) => {
            note(app, &st, format!("收到导入链接: {}", r.name.as_deref().unwrap_or("未命名订阅")));
            PendingLink { request: Some(r), error: None }
        }
        Err(err) => {
            note(app, &st, err.clone());
            PendingLink { request: None, error: Some(err) }
        }
    };
    st.imports.lock().unwrap().push(pending);
    let _ = app.emit("import-request", ());
    show_main(app);
}

#[tauri::command]
fn take_import_requests(st: State<'_, AppState>) -> Vec<PendingLink> {
    std::mem::take(&mut *st.imports.lock().unwrap())
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
async fn update_geodata(app: AppHandle, st: State<'_, AppState>) -> Res<()> {
    let port = st.proxy_port().await;
    download::geodata(&st.store.runtime_dir(), port).await.map_err(e)?;
    if st.settings().core == CoreKind::Singbox && st.settings().active_profile.is_some() {
        let failed = ensure_rule_sets(&app, &st, true).await?;
        if failed > 0 {
            return Err(format!("{failed} 个 sing-box 规则集下载失败，详见日志"));
        }
    }
    Ok(())
}

#[tauri::command]
fn preview_config(st: State<'_, AppState>) -> Res<String> {
    let cfg = core::write_runtime_config(&st.store, &st.settings()).map_err(e)?;
    std::fs::read_to_string(cfg.path).map_err(e)
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

/// A selectable group in the tray: its members with their last delay
/// (0 = timed out).
struct TrayGroup {
    name: String,
    now: String,
    members: Vec<(String, Option<u64>)>,
}

/// At most this many groups get a tray submenu.
const TRAY_GROUPS: usize = 8;
/// Separates group and node in a tray item id: `sel<SEP>group<SEP>node`.
const SEL_SEP: char = '\u{1}';

/// The running core's `select` groups, in the order the 代理 page shows them.
async fn tray_groups(s: &Settings) -> Vec<TrayGroup> {
    match core::api(s, "GET", "/proxies", None).await {
        Ok(v) => parse_tray_groups(&v, s.mode == Mode::Global),
        Err(_) => Vec::new(),
    }
}

/// Selectable groups from a `GET /proxies` response; `GLOBAL` leads in
/// global mode.
fn parse_tray_groups(v: &Value, global: bool) -> Vec<TrayGroup> {
    let px = &v["proxies"];
    let strings = |v: &Value| -> Vec<String> {
        v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
    };
    let mut order = strings(&px["GLOBAL"]["all"]);
    if global {
        order.insert(0, "GLOBAL".into());
    }
    order
        .into_iter()
        .filter(|n| px[n]["type"] == "Selector" && px[n]["hidden"] != true)
        .take(TRAY_GROUPS)
        .map(|n| TrayGroup {
            now: px[&n]["now"].as_str().unwrap_or_default().to_string(),
            members: strings(&px[&n]["all"])
                .into_iter()
                .map(|m| {
                    let d = px[&m]["history"].as_array().and_then(|h| h.last()).and_then(|r| r["delay"].as_u64());
                    (m, d)
                })
                .collect(),
            name: n,
        })
        .collect()
}

fn sel_id(group: &str, node: &str) -> String {
    format!("sel{SEL_SEP}{group}{SEL_SEP}{node}")
}

/// (group, node) from a tray item id made by [`sel_id`].
fn parse_sel_id(id: &str) -> Option<(&str, &str)> {
    id.strip_prefix("sel")?.strip_prefix(SEL_SEP)?.split_once(SEL_SEP)
}

/// Menu labels treat `&` as a mnemonic marker.
fn menu_text(s: &str) -> String {
    s.replace('&', "&&")
}

fn build_tray_menu(app: &AppHandle, groups: &[TrayGroup]) -> tauri::Result<Menu<tauri::Wry>> {
    let st = app.state::<AppState>();
    let s = st.settings();
    let en = s.lang == "en";
    let tr = |zh: &'static str, english: &'static str| if en { english } else { zh };
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(app, "show", tr("显示主界面", "Show Prism"), true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&CheckMenuItem::with_id(app, "sysproxy", tr("系统代理", "System proxy"), true, s.system_proxy, None::<&str>)?)?;
    menu.append(&CheckMenuItem::with_id(app, "tun", tr("TUN 模式", "TUN mode"), true, s.tun, None::<&str>)?)?;
    menu.append(&CheckMenuItem::with_id(app, "mode_rule", tr("规则模式", "Rule mode"), true, s.mode == Mode::Rule, None::<&str>)?)?;
    menu.append(&CheckMenuItem::with_id(app, "mode_global", tr("全局模式", "Global mode"), true, s.mode == Mode::Global, None::<&str>)?)?;
    menu.append(&CheckMenuItem::with_id(app, "mode_direct", tr("直连模式", "Direct mode"), true, s.mode == Mode::Direct, None::<&str>)?)?;
    if !groups.is_empty() {
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        for g in groups {
            let sub = Submenu::new(app, menu_text(&format!("{}：{}", g.name, g.now)), true)?;
            for (m, d) in &g.members {
                let delay = match d {
                    Some(0) => format!("\t{}", tr("超时", "timeout")),
                    Some(ms) => format!("\t{ms} ms"),
                    None => String::new(),
                };
                sub.append(&CheckMenuItem::with_id(app, sel_id(&g.name, m), format!("{}{delay}", menu_text(m)), true, *m == g.now, None::<&str>)?)?;
            }
            menu.append(&sub)?;
        }
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "quit", tr("退出", "Quit"), true, None::<&str>)?)?;
    Ok(menu)
}

/// After delay tests, so the tray shows the new delays.
#[tauri::command]
fn update_tray(app: AppHandle) {
    refresh_tray(&app);
}

/// Rebuilds the tray menu, including the running core's groups.
fn refresh_tray(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let st = app.state::<AppState>();
        let _guard = st.tray_lock.lock().await;
        let groups = match st.core.running().await {
            Some(_) => tray_groups(&st.settings()).await,
            None => Vec::new(),
        };
        match (app.tray_by_id("main"), build_tray_menu(&app, &groups)) {
            (Some(tray), Ok(menu)) => {
                let _ = tray.set_menu(Some(menu));
            }
            (_, Err(err)) => eprintln!("[prism] 托盘菜单生成失败: {err}"),
            _ => {}
        }
    });
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
            "tun" => {
                let on = !st.settings().tun;
                set_tun(app.clone(), st.clone(), on).await
            }
            "mode_rule" | "mode_global" | "mode_direct" => {
                set_mode(app.clone(), st.clone(), id.trim_start_matches("mode_").to_string()).await
            }
            "quit" => {
                app.exit(0);
                Ok(())
            }
            _ => match parse_sel_id(&id) {
                Some((group, node)) => select_proxy(app.clone(), st.clone(), group.into(), node.into()).await,
                None => Ok(()),
            },
        };
        if let Err(err) = r {
            st.core.push_log(&app, format!("[prism] {err}"));
        }
        let _ = app.emit("core-changed", ());
        refresh_tray(&app);
    });
}

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    // After "restart as administrator", let the old instance exit first: it
    // holds the single-instance lock and the core's ports.
    let wait = args.iter().position(|a| a == elevate::WAIT_ARG).and_then(|i| args.get(i + 1));
    if let Some(pid) = wait.and_then(|p| p.parse().ok()) {
        elevate::wait_for_exit(pid, Duration::from_secs(15));
    }
    let autostarted = args.iter().any(|a| a == autostart::ARG);
    let link = links::find(args.iter().skip(1).cloned());

    let app = tauri::Builder::default()
        // Must come first: a second launch hands its arguments (an import
        // link, say) to the running Prism and exits.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| match links::find(argv.into_iter().skip(1)) {
            Some(link) => handle_link(app, &link),
            None => show_main(app),
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, sc, ev| {
                    if ev.state != tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        return;
                    }
                    let action = app.try_state::<AppState>().and_then(|st| st.hotkeys.lock().unwrap().get(&sc.id()).cloned());
                    if let Some(a) = action {
                        on_hotkey(app, &a);
                    }
                })
                .build(),
        )
        .setup(move |app| {
            // PRISM_DATA_DIR: a separate data folder (portable use, tests).
            let dir = match std::env::var_os("PRISM_DATA_DIR") {
                Some(d) => std::path::PathBuf::from(d),
                None => app.path().app_data_dir()?,
            };
            let store = Store::new(dir)?;
            let settings = store.settings();
            let traffic = traffic::Traffic::load(store.dir.join("traffic.json"));
            let hidden = autostarted && settings.silent_start && link.is_none();
            app.manage(AppState {
                store,
                settings: Mutex::new(settings),
                core: CoreManager::default(),
                update: Mutex::new(None),
                imports: Mutex::new(Vec::new()),
                tray_lock: Default::default(),
                traffic,
                hotkeys: Default::default(),
            });
            {
                let s = app.state::<AppState>().settings();
                apply_theme(app.handle(), &s.theme);
                for f in apply_hotkeys(app.handle(), &s) {
                    eprintln!("[prism] hotkey not registered: {f}");
                }
            }
            tauri::async_runtime::spawn(traffic_sampler(app.handle().clone()));

            let menu = build_tray_menu(app.handle(), &[])?;
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

            // The window starts hidden (tauri.conf.json) so a start with
            // Windows can stay in the tray.
            if !hidden {
                show_main(app.handle());
            }
            if let Some(link) = &link {
                handle_link(app.handle(), link);
            }
            if !cfg!(debug_assertions) {
                // Installed builds own prism://; dev builds leave it alone.
                if let Err(err) = links::register("prism") {
                    eprintln!("[prism] 注册 prism:// 链接失败: {err:#}");
                }
            }
            tauri::async_runtime::spawn(subscription_scheduler(app.handle().clone()));

            // Auto-start the core when a profile is configured.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let st = handle.state::<AppState>();
                let mut s = st.settings();
                if s.active_profile.is_some() {
                    if s.tun && !tun_allowed() {
                        // Started with Windows, say: run without TUN rather than not at all.
                        s = st.update(|x| x.tun = false).unwrap_or(s);
                        note(&handle, &st, "TUN 模式需要管理员权限，本次未开启 TUN（可安装 TUN 服务，或以管理员身份重启）".into());
                    }
                    let mut r = start(&handle, &st).await;
                    if matches!(&r, Err(err) if err.starts_with("NEED_SERVICE_UPDATE")) {
                        // No UAC prompt at logon: run without TUN this time.
                        s = st.update(|x| x.tun = false).unwrap_or(s);
                        note(&handle, &st, "TUN 服务中的内核需要更新，本次未开启 TUN（在设置页更新 TUN 服务后重新打开）".into());
                        r = start(&handle, &st).await;
                    }
                    match r {
                        Ok(()) if s.system_proxy => {
                            let _ = sysproxy::set(true, &format!("127.0.0.1:{}", s.mixed_port), &s.proxy_bypass);
                        }
                        Ok(()) => {}
                        Err(err) => st.core.push_log(&handle, format!("[prism] 自动启动失败: {err}")),
                    }
                }
                if s.auto_update_check {
                    // Give the core a moment so the check can use its proxy.
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    match updater::find(&handle, st.proxy_port().await).await {
                        Ok(Some(u)) => {
                            note(&handle, &st, format!("发现新版本 v{}（当前 v{}）", u.version, u.current_version));
                            let _ = handle.emit("update-available", updater::info(&u));
                            *st.update.lock().unwrap() = Some(u);
                        }
                        Ok(None) => note(&handle, &st, "已是最新版本".into()),
                        Err(err) => note(&handle, &st, format!("检查更新失败: {err:#}")),
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
            set_tun,
            restart_as_admin,
            get_logs,
            list_processes,
            profile_outbounds,
            set_app_rules,
            set_profile_interval,
            set_autostart,
            set_clash_links,
            take_import_requests,
            update_tray,
            traffic_history,
            unlock_check,
            get_patches,
            save_patches,
            webdav_save,
            webdav_test,
            backup_upload,
            backup_list,
            backup_delete,
            backup_restore,
            backup_import,
            backup_export,
            install_tun_service,
            uninstall_tun_service,
            check_update,
            install_update,
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
            st.traffic.save();
            if st.settings().system_proxy {
                let _ = sysproxy::set(false, "", "");
            }
            tauri::async_runtime::block_on(st.core.stop());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_headers() {
        assert_eq!(disposition_name(r#"attachment; filename="sbm.yaml""#).as_deref(), Some("sbm.yaml"));
        assert_eq!(disposition_name("attachment;filename=plain").as_deref(), Some("plain"));
        assert_eq!(
            disposition_name("attachment; filename=fallback.yaml; filename*=UTF-8''%E6%88%91%E7%9A%84%E6%9C%BA%E5%9C%BA").as_deref(),
            Some("我的机场")
        );
        assert_eq!(disposition_name("inline"), None);
        assert_eq!(disposition_name("attachment; filename="), None);

        assert_eq!(interval_minutes("24"), Some(1440));
        assert_eq!(interval_minutes(" 0.5 "), Some(30));
        assert_eq!(interval_minutes("0"), Some(0));
        assert_eq!(interval_minutes("-1"), None);
        assert_eq!(interval_minutes("soon"), None);
    }

    #[test]
    fn tray_groups_follow_the_proxies_page() {
        let v = json!({ "proxies": {
            "GLOBAL": { "type": "Selector", "now": "节点选择", "all": ["节点选择", "自动选择", "隐藏组", "流媒体"] },
            "节点选择": { "type": "Selector", "now": "东京", "all": ["自动选择", "东京", "A&B", "DIRECT"] },
            "自动选择": { "type": "URLTest", "now": "东京", "all": ["东京", "A&B"] },
            "隐藏组": { "type": "Selector", "hidden": true, "now": "东京", "all": ["东京"] },
            "流媒体": { "type": "Selector", "now": "A&B", "all": ["A&B"] },
            "东京": { "type": "AnyTLS", "history": [{ "delay": 0 }, { "delay": 120 }] },
            "A&B": { "type": "Shadowsocks", "history": [{ "delay": 0 }] },
            "DIRECT": { "type": "Direct", "history": [] },
        }});
        let names = |g: &[TrayGroup]| g.iter().map(|g| g.name.clone()).collect::<Vec<_>>();
        // URL-test and hidden groups can't be picked from the tray.
        let rule = parse_tray_groups(&v, false);
        assert_eq!(names(&rule), ["节点选择", "流媒体"]);
        assert_eq!(rule[0].now, "东京");
        assert_eq!(
            rule[0].members,
            [("自动选择".to_string(), None), ("东京".into(), Some(120)), ("A&B".into(), Some(0)), ("DIRECT".into(), None)]
        );
        assert_eq!(names(&parse_tray_groups(&v, true)), ["GLOBAL", "节点选择", "流媒体"]);
        assert!(parse_tray_groups(&json!({}), false).is_empty());

        let id = sel_id("节点选择", "A&B 01");
        assert_eq!(parse_sel_id(&id), Some(("节点选择", "A&B 01")));
        assert_eq!(parse_sel_id("sysproxy"), None);
        assert_eq!(parse_sel_id("selx"), None);
        assert_eq!(menu_text("A&B"), "A&&B");
    }
}
