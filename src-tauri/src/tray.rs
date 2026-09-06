//! System tray: icon, menu (show / mode switch / system proxy / quit) and
//! check-state syncing with the frontend.

use std::sync::Mutex;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

pub struct Items {
    show: MenuItem<Wry>,
    rule: CheckMenuItem<Wry>,
    global: CheckMenuItem<Wry>,
    direct: CheckMenuItem<Wry>,
    sysproxy: CheckMenuItem<Wry>,
    quit: MenuItem<Wry>,
}

/// Menu labels per locale: [show, rule, global, direct, sysproxy, quit]
fn labels(locale: &str) -> [&'static str; 6] {
    match locale {
        "en" => ["Show Mfork", "Rule Mode", "Global Mode", "Direct Mode", "System Proxy", "Quit"],
        "zh-TW" => ["顯示 Mfork", "規則模式", "全局模式", "直連模式", "系統代理", "退出"],
        _ => ["显示 Mfork", "规则模式", "全局模式", "直连模式", "系统代理", "退出"],
    }
}

#[derive(Default)]
pub struct TrayState(pub Mutex<Option<Items>>);

/// Last proxy mode reported by the frontend; None = core not running.
static LAST_MODE: Mutex<Option<String>> = Mutex::new(None);

pub fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    let settings = crate::settings::load(app);
    let l = labels(&settings.locale);
    let show = MenuItem::with_id(app, "show", l[0], true, None::<&str>)?;
    let rule = CheckMenuItem::with_id(app, "mode-rule", l[1], true, false, None::<&str>)?;
    let global = CheckMenuItem::with_id(app, "mode-global", l[2], true, false, None::<&str>)?;
    let direct = CheckMenuItem::with_id(app, "mode-direct", l[3], true, false, None::<&str>)?;
    let sysproxy = CheckMenuItem::with_id(
        app,
        "sys-proxy",
        l[4],
        true,
        settings.system_proxy,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", l[5], true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &show,
            &PredefinedMenuItem::separator(app)?,
            &rule,
            &global,
            &direct,
            &PredefinedMenuItem::separator(app)?,
            &sysproxy,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Mfork")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_window(app),
            "quit" => app.exit(0), // RunEvent::Exit clears the proxy and kills the core
            "sys-proxy" => {
                let enabled = crate::settings::load(app).system_proxy;
                let _ = crate::system::set_system_proxy_inner(app, !enabled);
            }
            id @ ("mode-rule" | "mode-global" | "mode-direct") => {
                // the frontend performs the switch (it owns the mihomo API
                // connection) and calls tray_sync back on success
                let _ = app.emit("tray-mode", id.trim_start_matches("mode-").to_string());
                refresh(app);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;

    *app.state::<TrayState>().0.lock().unwrap() = Some(Items {
        show,
        rule,
        global,
        direct,
        sysproxy,
        quit,
    });
    Ok(())
}

pub fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Relabel the menu for a new UI language.
pub fn set_locale(app: &AppHandle, locale: &str) {
    let l = labels(locale);
    let state = app.state::<TrayState>();
    let guard = state.0.lock().unwrap();
    if let Some(items) = guard.as_ref() {
        let _ = items.show.set_text(l[0]);
        let _ = items.rule.set_text(l[1]);
        let _ = items.global.set_text(l[2]);
        let _ = items.direct.set_text(l[3]);
        let _ = items.sysproxy.set_text(l[4]);
        let _ = items.quit.set_text(l[5]);
    }
}

/// Re-apply check states from settings + the last known mode.
pub fn refresh(app: &AppHandle) {
    let mode = LAST_MODE.lock().unwrap().clone();
    let sys = crate::settings::load(app).system_proxy;
    let state = app.state::<TrayState>();
    let guard = state.0.lock().unwrap();
    if let Some(items) = guard.as_ref() {
        let m = mode.as_deref();
        let _ = items.rule.set_checked(m == Some("rule"));
        let _ = items.global.set_checked(m == Some("global"));
        let _ = items.direct.set_checked(m == Some("direct"));
        let _ = items.sysproxy.set_checked(sys);
    }
}

/// Frontend reports the current proxy mode (null = core stopped).
#[tauri::command]
pub fn tray_sync(app: AppHandle, mode: Option<String>) {
    *LAST_MODE.lock().unwrap() = mode;
    refresh(&app);
}
