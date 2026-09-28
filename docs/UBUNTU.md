# Ubuntu 26.04

Mfork uses Tauri 2 / GTK 3 / WebKitGTK 4.1. Build natively on Ubuntu 26.04
for the machine's architecture. A package built here is not intended for
older Ubuntu versions with older glibc.

Source baseline and history correction: [0.1.11 Ubuntu release notes](releases/0.1.11-ubuntu.md).

The desktop and tray icons use the approved “Fork” mark from
`design/Main.dc.html` and `src/components/icons.tsx`, exported as
`design/mfork-icon.svg`. Desktop PNG/ICO/ICNS assets live in `src-tauri/icons/`.
After reinstalling a package with updated icons, fully quit and restart
Mfork to refresh the embedded tray icon.

## Build

```sh
sudo apt update
sudo apt install -y build-essential pkg-config curl wget file \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
  libssl-dev patchelf nodejs npm rustc cargo
npm install --global pnpm@10 --prefix "$HOME/.local"
export PATH="$HOME/.local/bin:$PATH"
pnpm install --frozen-lockfile
pnpm build:ubuntu
```

The build downloads the official mihomo core, compiles the systemd service,
and bundles both into `src-tauri/target/release/bundle/deb/`. The Ubuntu
configuration disables updater artifact signing for local .deb builds;
the regular release configuration remains available for signed releases.

## Install and run

```sh
sudo apt install ./src-tauri/target/release/bundle/deb/Mfork_0.1.11_amd64.deb
mfork
```

Run the desktop app as your normal user. Import a subscription or local
configuration before connecting. System proxy controls the desktop proxy;
programs that ignore GNOME proxy settings may need a proxy configured
explicitly or TUN mode.

For TUN, use Settings to install the Mfork service and accept the polkit
password prompt. Then enable TUN. The helper installs
`mfork-service.service` and a root-owned copy of mihomo; the desktop app
itself does not need root.

Closing the window keeps the application in the tray. Use the tray menu to
show it or quit. Launching Mfork again also shows the existing window.
GNOME needs the Ubuntu AppIndicators extension enabled for the tray icon.

```sh
systemctl status mfork-service
journalctl -u mfork-service -b --no-pager
```

Before removing the application, uninstall the TUN service in Settings.
Update this .deb installation by installing a newer .deb; the existing
in-app updater release flow targets AppImage on Linux.

## Validation on Ubuntu 26.04

- Check package dependencies and included mihomo/service executables.
- Launch in a desktop session and check rendering and window controls.
- Import a configuration, start/stop the core, and check system proxy.
- Check tray show/quit and a second launch.
- Install the service with polkit, enable TUN, and verify DNS and routing.
- Quit and verify the core exits and the desktop proxy is disabled.

Build success alone does not verify desktop integration or TUN routing.

### Verified build: 0.1.11 (2026-09-28)

On Ubuntu 26.04 x86_64 with Node 22.22.1, Rust 1.93.1 and WebKitGTK 2.52.6:

- TypeScript and Vite production build passed.
- Native desktop application and systemd helper compiled successfully.
- All 15 Rust unit tests passed (`cargo test --release --locked --manifest-path src-tauri/Cargo.toml --lib`).
- Bundled mihomo v1.19.31 passed configuration validation and a local `/version` API smoke test.
- `.deb` includes the executable, desktop launcher, icons, mihomo and helper.
- No missing shared libraries were reported for the desktop executable.
- `apt-get -s install ./src-tauri/target/release/bundle/deb/Mfork_0.1.11_amd64.deb` resolved successfully.

Desktop interaction, subscription connectivity, polkit authorization and TUN
routing have not yet been verified in an interactive installed session.
