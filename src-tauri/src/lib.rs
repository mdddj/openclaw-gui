use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WindowEvent,
};

const GATEWAY_COMMAND_LABEL: &str = "openclaw gateway";
const DASHBOARD_COMMAND_LABEL: &str = "openclaw dashboard";
const GATEWAY_LOG_EVENT: &str = "gateway-log";
const GATEWAY_STATE_EVENT: &str = "gateway-state";
const LOG_LIMIT: usize = 800;

const TRAY_SHOW_ID: &str = "tray.show";
const TRAY_START_ID: &str = "tray.start";
const TRAY_STOP_ID: &str = "tray.stop";
const TRAY_RESTART_ID: &str = "tray.restart";
const TRAY_QUIT_ID: &str = "tray.quit";

#[derive(Clone, Default)]
struct GatewayState {
    inner: Arc<GatewayStateInner>,
}

#[derive(Default)]
struct GatewayStateInner {
    state: Mutex<GatewayRuntimeState>,
    log_sequence: AtomicU64,
}

#[derive(Default)]
struct GatewayRuntimeState {
    runtime: Option<GatewayProcess>,
    last_error: Option<String>,
    last_exit: Option<GatewayExitInfo>,
    logs: VecDeque<GatewayLogEntry>,
}

struct GatewayProcess {
    child: Child,
    pid: u32,
    started_at_ms: u64,
}

#[derive(Default)]
struct AppLifecycleState {
    quitting: AtomicBool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum GatewayStatus {
    Stopped,
    Running,
}

impl Default for GatewayStatus {
    fn default() -> Self {
        Self::Stopped
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GatewayExitInfo {
    at_ms: u64,
    code: Option<i32>,
    success: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GatewayLogEntry {
    id: u64,
    timestamp_ms: u64,
    stream: String,
    message: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GatewayStatePayload {
    command: &'static str,
    status: GatewayStatus,
    pid: Option<u32>,
    started_at_ms: Option<u64>,
    last_error: Option<String>,
    last_exit: Option<GatewayExitInfo>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GatewaySnapshot {
    #[serde(flatten)]
    state: GatewayStatePayload,
    logs: Vec<GatewayLogEntry>,
}

impl GatewayState {
    fn snapshot(&self) -> GatewaySnapshot {
        let state = self.inner.state.lock().expect("gateway state poisoned");
        Self::snapshot_from_locked(&state)
    }

    fn start(&self, app: &AppHandle) -> Result<GatewaySnapshot, String> {
        {
            let state = self.inner.state.lock().expect("gateway state poisoned");
            if state.runtime.is_some() {
                return Ok(Self::snapshot_from_locked(&state));
            }
        }

        let mut command = gateway_command();
        let spawn_result = command.spawn();

        let (stdout, stderr, pid) = match spawn_result {
            Ok(mut child) => {
                let pid = child.id();
                let stdout = child.stdout.take();
                let stderr = child.stderr.take();

                {
                    let mut state = self.inner.state.lock().expect("gateway state poisoned");
                    state.last_error = None;
                    state.last_exit = None;
                    state.runtime = Some(GatewayProcess {
                        child,
                        pid,
                        started_at_ms: now_ms(),
                    });
                }

                (stdout, stderr, pid)
            }
            Err(error) => {
                let message = format!("启动 `{GATEWAY_COMMAND_LABEL}` 失败: {error}");
                {
                    let mut state = self.inner.state.lock().expect("gateway state poisoned");
                    state.last_error = Some(message.clone());
                    state.last_exit = None;
                    state.runtime = None;
                }
                self.push_log(app, "system", message.clone());
                self.emit_state(app);
                return Err(message);
            }
        };

        if let Some(stdout) = stdout {
            spawn_output_reader(app.clone(), self.clone(), "stdout", stdout);
        }
        if let Some(stderr) = stderr {
            spawn_output_reader(app.clone(), self.clone(), "stderr", stderr);
        }

        self.push_log(app, "system", format!("已启动网关进程，PID {pid}。"));
        self.emit_state(app);

        Ok(self.snapshot())
    }

    fn stop(&self, app: &AppHandle) -> Result<GatewaySnapshot, String> {
        let runtime = {
            let mut state = self.inner.state.lock().expect("gateway state poisoned");
            match state.runtime.take() {
                Some(runtime) => runtime,
                None => return Ok(Self::snapshot_from_locked(&state)),
            }
        };

        let (last_exit, last_error, log_message) = stop_gateway_process(runtime);

        {
            let mut state = self.inner.state.lock().expect("gateway state poisoned");
            state.last_exit = last_exit;
            state.last_error = last_error.clone();
            state.runtime = None;
        }

        self.push_log(app, "system", log_message);
        self.emit_state(app);

        match last_error {
            Some(error) => Err(error),
            None => Ok(self.snapshot()),
        }
    }

    fn restart(&self, app: &AppHandle) -> Result<GatewaySnapshot, String> {
        let _ = self.stop(app);
        self.push_log(app, "system", "正在重新启动网关进程。");
        self.start(app)
    }

    fn emit_state(&self, app: &AppHandle) {
        let _ = app.emit(GATEWAY_STATE_EVENT, self.state_payload());
    }

    fn push_log<S: Into<String>>(&self, app: &AppHandle, stream: &str, message: S) {
        let entry = {
            let mut state = self.inner.state.lock().expect("gateway state poisoned");
            let entry = GatewayLogEntry {
                id: self.inner.log_sequence.fetch_add(1, Ordering::Relaxed) + 1,
                timestamp_ms: now_ms(),
                stream: stream.to_string(),
                message: message.into(),
            };

            if state.logs.len() >= LOG_LIMIT {
                state.logs.pop_front();
            }
            state.logs.push_back(entry.clone());
            entry
        };

        let _ = app.emit(GATEWAY_LOG_EVENT, entry);
    }

    fn sync_process_exit(&self, app: &AppHandle, exit_status: Result<Option<ExitStatus>, String>) {
        let message = {
            let mut state = self.inner.state.lock().expect("gateway state poisoned");
            if state.runtime.is_none() {
                return;
            }

            match exit_status {
                Ok(Some(status)) => {
                    let exit_info = exit_info(status);
                    state.last_exit = Some(exit_info.clone());
                    state.last_error = if exit_info.success {
                        None
                    } else {
                        Some(format!("网关进程已退出（{}）。", describe_exit(&exit_info)))
                    };
                    state.runtime = None;

                    if exit_info.success {
                        "网关进程已退出。".to_string()
                    } else {
                        format!("网关进程异常退出（{}）。", describe_exit(&exit_info))
                    }
                }
                Ok(None) => return,
                Err(error) => {
                    state.last_error = Some(error.clone());
                    state.last_exit = None;
                    state.runtime = None;
                    error
                }
            }
        };

        self.push_log(app, "system", message);
        self.emit_state(app);
    }

    fn state_payload(&self) -> GatewayStatePayload {
        let state = self.inner.state.lock().expect("gateway state poisoned");
        Self::state_payload_from_locked(&state)
    }

    fn state_payload_from_locked(state: &GatewayRuntimeState) -> GatewayStatePayload {
        GatewayStatePayload {
            command: GATEWAY_COMMAND_LABEL,
            status: if state.runtime.is_some() {
                GatewayStatus::Running
            } else {
                GatewayStatus::Stopped
            },
            pid: state.runtime.as_ref().map(|runtime| runtime.pid),
            started_at_ms: state.runtime.as_ref().map(|runtime| runtime.started_at_ms),
            last_error: state.last_error.clone(),
            last_exit: state.last_exit.clone(),
        }
    }

    fn snapshot_from_locked(state: &GatewayRuntimeState) -> GatewaySnapshot {
        GatewaySnapshot {
            state: Self::state_payload_from_locked(state),
            logs: state.logs.iter().cloned().collect(),
        }
    }
}

impl AppLifecycleState {
    fn mark_quitting(&self) {
        self.quitting.store(true, Ordering::Relaxed);
    }

    fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::Relaxed)
    }
}

#[tauri::command]
fn get_gateway_snapshot(state: State<'_, GatewayState>) -> GatewaySnapshot {
    state.snapshot()
}

#[tauri::command]
fn start_gateway(
    app: AppHandle,
    state: State<'_, GatewayState>,
) -> Result<GatewaySnapshot, String> {
    state.start(&app)
}

#[tauri::command]
fn stop_gateway(app: AppHandle, state: State<'_, GatewayState>) -> Result<GatewaySnapshot, String> {
    state.stop(&app)
}

#[tauri::command]
fn restart_gateway(
    app: AppHandle,
    state: State<'_, GatewayState>,
) -> Result<GatewaySnapshot, String> {
    state.restart(&app)
}

#[tauri::command]
fn open_dashboard(app: AppHandle, state: State<'_, GatewayState>) -> Result<(), String> {
    match dashboard_command().spawn() {
        Ok(_) => {
            state.push_log(&app, "system", format!("已执行 `{DASHBOARD_COMMAND_LABEL}`。"));
            Ok(())
        }
        Err(error) => {
            let message = format!("执行 `{DASHBOARD_COMMAND_LABEL}` 失败: {error}");
            state.push_log(&app, "system", message.clone());
            Err(message)
        }
    }
}

fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, TRAY_SHOW_ID, "显示主窗口", true, None::<&str>)?;
    let start = MenuItem::with_id(app, TRAY_START_ID, "启动网关", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, TRAY_STOP_ID, "停止网关", true, None::<&str>)?;
    let restart = MenuItem::with_id(app, TRAY_RESTART_ID, "重新启动网关", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, TRAY_QUIT_ID, "退出", true, None::<&str>)?;
    let separator_top = PredefinedMenuItem::separator(app)?;
    let separator_bottom = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(
        app,
        &[
            &show,
            &separator_top,
            &start,
            &stop,
            &restart,
            &separator_bottom,
            &quit,
        ],
    )?;

    let mut tray = TrayIconBuilder::with_id("gateway-tray")
        .menu(&menu)
        .tooltip("OpenClaw Gateway");

    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }

    tray.on_menu_event(|app, event| match event.id().as_ref() {
        TRAY_SHOW_ID => show_main_window(app),
        TRAY_START_ID => {
            let gateway = app.state::<GatewayState>().inner().clone();
            let _ = gateway.start(app);
        }
        TRAY_STOP_ID => {
            let gateway = app.state::<GatewayState>().inner().clone();
            let _ = gateway.stop(app);
        }
        TRAY_RESTART_ID => {
            let gateway = app.state::<GatewayState>().inner().clone();
            let _ = gateway.restart(app);
        }
        TRAY_QUIT_ID => {
            app.state::<AppLifecycleState>().mark_quitting();
            let gateway = app.state::<GatewayState>().inner().clone();
            let _ = gateway.stop(app);
            app.exit(0);
        }
        _ => {}
    })
    .build(app)?;

    Ok(())
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn spawn_gateway_monitor(app: AppHandle, gateway: GatewayState) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(1));

        let exit_status = {
            let mut state = gateway.inner.state.lock().expect("gateway state poisoned");
            match state.runtime.as_mut() {
                Some(runtime) => runtime
                    .child
                    .try_wait()
                    .map_err(|error| format!("检查网关进程状态失败: {error}")),
                None => continue,
            }
        };

        gateway.sync_process_exit(&app, exit_status);
    });
}

fn spawn_output_reader<R: Read + Send + 'static>(
    app: AppHandle,
    gateway: GatewayState,
    stream: &'static str,
    reader: R,
) {
    thread::spawn(move || {
        let buffered = BufReader::new(reader);

        for line in buffered.lines() {
            match line {
                Ok(line) if !line.trim().is_empty() => gateway.push_log(&app, stream, line),
                Ok(_) => {}
                Err(error) => {
                    gateway.push_log(&app, "system", format!("读取 {stream} 日志失败: {error}"));
                    break;
                }
            }
        }
    });
}

fn gateway_command() -> Command {
    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("cmd");
        command.args([
            "/C",
            "where openclaw >nul 2>nul || (echo openclaw command not found in PATH 1>&2 & exit /b 127) && openclaw gateway",
        ]);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        return command;
    }

    #[cfg(not(target_os = "windows"))]
    {
        let mut command = Command::new("/bin/zsh");
        command.args([
            "-lc",
            "command -v openclaw >/dev/null 2>&1 || { echo 'openclaw command not found in PATH' >&2; exit 127; }; exec openclaw gateway",
        ]);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command
    }
}

fn dashboard_command() -> Command {
    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("cmd");
        command.args([
            "/C",
            "where openclaw >nul 2>nul || (echo openclaw command not found in PATH 1>&2 & exit /b 127) && openclaw dashboard",
        ]);
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        return command;
    }

    #[cfg(not(target_os = "windows"))]
    {
        let mut command = Command::new("/bin/zsh");
        command.args([
            "-lc",
            "command -v openclaw >/dev/null 2>&1 || { echo 'openclaw command not found in PATH' >&2; exit 127; }; exec openclaw dashboard",
        ]);
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        command
    }
}

fn stop_gateway_process(
    runtime: GatewayProcess,
) -> (Option<GatewayExitInfo>, Option<String>, String) {
    let mut child = runtime.child;

    match child.try_wait() {
        Ok(Some(status)) => {
            let exit_info = exit_info(status);
            let message = if exit_info.success {
                "网关进程已经停止。".to_string()
            } else {
                format!("网关进程已停止（{}）。", describe_exit(&exit_info))
            };
            let error = if exit_info.success {
                None
            } else {
                Some(format!("网关进程已退出（{}）。", describe_exit(&exit_info)))
            };
            (Some(exit_info), error, message)
        }
        Ok(None) => {
            let _ = child.kill();
            match child.wait() {
                Ok(status) => (
                    Some(exit_info(status)),
                    None,
                    format!("已停止网关进程，PID {}。", runtime.pid),
                ),
                Err(error) => {
                    let message = format!("停止网关进程失败: {error}");
                    (None, Some(message.clone()), message)
                }
            }
        }
        Err(error) => {
            let message = format!("检查网关进程状态失败: {error}");
            (None, Some(message.clone()), message)
        }
    }
}

fn exit_info(status: ExitStatus) -> GatewayExitInfo {
    GatewayExitInfo {
        at_ms: now_ms(),
        code: status.code(),
        success: status.success(),
    }
}

fn describe_exit(info: &GatewayExitInfo) -> String {
    match info.code {
        Some(code) => format!("退出码 {code}"),
        None => "无退出码".to_string(),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .manage(GatewayState::default())
        .manage(AppLifecycleState::default())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            setup_tray(app)?;

            let app_handle = app.handle().clone();
            let gateway = app.state::<GatewayState>().inner().clone();
            spawn_gateway_monitor(app_handle.clone(), gateway.clone());

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }

            if let WindowEvent::CloseRequested { api, .. } = event {
                let lifecycle = window.state::<AppLifecycleState>();
                if lifecycle.is_quitting() {
                    return;
                }

                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_gateway_snapshot,
            start_gateway,
            stop_gateway,
            restart_gateway,
            open_dashboard
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle: &AppHandle, event| {
        if let tauri::RunEvent::ExitRequested { .. } = event {
            app_handle.state::<AppLifecycleState>().mark_quitting();
            let gateway = app_handle.state::<GatewayState>().inner().clone();
            let _ = gateway.stop(app_handle);
        }
    });
}
