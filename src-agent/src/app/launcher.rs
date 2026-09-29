//! App-list entry and icon for Linux and macOS.
//!
//! `koma update` and `install.sh` both end in `koma launcher-install`. The icon
//! bytes are compiled into the binary, so placement does not depend on a second
//! download. Linux writes a user desktop file plus hicolor PNGs. macOS writes
//! `~/Applications/Koma.app` whose executable is this binary, so Launchpad and
//! the Dock use `AppIcon.icns`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

const ICON_32: &[u8] = include_bytes!("../../../assets/icon-32.png");
const ICON_48: &[u8] = include_bytes!("../../../assets/icon-48.png");
const ICON_64: &[u8] = include_bytes!("../../../assets/icon-64.png");
const ICON_128: &[u8] = include_bytes!("../../../assets/icon-128.png");
pub(crate) const ICON_256: &[u8] = include_bytes!("../../../assets/icon-256.png");
const ICON_512: &[u8] = include_bytes!("../../../assets/icon-512.png");
const ICON_ICNS: &[u8] = include_bytes!("../../../assets/icon.icns");

const LINUX_SIZES: &[(u32, &[u8])] = &[
    (32, ICON_32),
    (48, ICON_48),
    (64, ICON_64),
    (128, ICON_128),
    (256, ICON_256),
    (512, ICON_512),
];

/// Where the shortcut was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub location: PathBuf,
}

/// Write the current platform's app-list entry and icon.
pub fn install() -> Result<Installed> {
    let home = dirs::home_dir().context("HOME is not set")?;
    let exe = installed_exe().context("could not find the koma binary to launch")?;
    install_at(&home, &exe, true)
}

/// Finder launches `Koma.app` with no subcommand. Info.plist cannot append
/// `gui`, so the bundle executable opens the desktop client itself.
pub fn args_for_launch(args: Vec<String>) -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        let mut args = args;
        if should_launch_gui(&args) {
            args.insert(1, "gui".to_string());
        }
        return args;
    }
    #[cfg(not(target_os = "macos"))]
    {
        args
    }
}

/// True when this process was started as `Koma.app` with no subcommand.
#[cfg(target_os = "macos")]
pub fn should_launch_gui(args: &[String]) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    exe_should_launch_gui(&exe, args)
}

pub fn exe_should_launch_gui(exe: &Path, args: &[String]) -> bool {
    if !is_app_bundle_exe(exe) {
        return false;
    }
    args.iter().skip(1).all(|arg| arg.starts_with("-psn_"))
}

/// PNG path written for the macOS Dock, once [`install`] has run.
#[cfg(target_os = "macos")]
pub fn dock_icon_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let path = home.join("Applications/Koma.app/Contents/Resources/icon-256.png");
    path.is_file().then_some(path)
}

fn install_at(home: &Path, exe: &Path, register: bool) -> Result<Installed> {
    if cfg!(target_os = "macos") {
        install_macos(home, exe, register)
    } else if cfg!(target_os = "linux") {
        install_linux(home, &data_home(home), exe, register)
    } else {
        anyhow::bail!("app list placement supports Linux and macOS")
    }
}

fn installed_exe() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::var_os("KOMA_INSTALL_DIR") {
        candidates.push(PathBuf::from(dir).join("koma"));
    }
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local/bin/koma"));
        candidates.push(home.join(".local/bin/koma.bin"));
    }
    if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
        return Some(linux_launch_exe(&path));
    }
    std::env::current_exe()
        .ok()
        .filter(|path| path.is_file())
        .map(|path| linux_launch_exe(&path))
}

/// The curl installer's `koma` is a shell wrapper around `koma.bin`. The
/// desktop entry must exec the wrapper so the library preflight still runs.
fn linux_launch_exe(exe: &Path) -> PathBuf {
    if exe.file_name() == Some(std::ffi::OsStr::new("koma.bin")) {
        let wrapper = exe.with_file_name("koma");
        if wrapper.is_file() {
            return wrapper;
        }
    }
    exe.to_path_buf()
}

/// GTK's window class is the executable's file name. The wrapper `exec`s
/// `koma.bin`, so that is the class a terminal launch actually has. A `.deb`
/// installs the ELF as `koma`.
fn wm_class(launch_exe: &Path) -> &'static str {
    if launch_exe.file_name() == Some(std::ffi::OsStr::new("koma")) {
        let bin = launch_exe.with_file_name("koma.bin");
        if bin.is_file() {
            return "koma.bin";
        }
    }
    "koma"
}

fn data_home(home: &Path) -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    home.join(".local/share")
}

fn install_linux(home: &Path, data_home: &Path, exe: &Path, register: bool) -> Result<Installed> {
    let _ = home;
    let icons = data_home.join("icons/hicolor");
    let mut icon_file = None;
    for (size, bytes) in LINUX_SIZES {
        let dest = icons.join(format!("{size}x{size}/apps/koma.png"));
        write_bytes(&dest, bytes)?;
        if *size == 256 {
            icon_file = Some(dest);
        }
    }
    let icon_file = icon_file.context("256px icon was not written")?;
    let apps = data_home.join("applications");
    let desktop = apps.join("koma.desktop");
    let body = linux_desktop(exe, &icon_file, wm_class(exe));
    write_bytes(&desktop, body.as_bytes())?;
    if register {
        register_linux(&apps, &icons, &desktop);
    }
    Ok(Installed { location: desktop })
}

fn linux_desktop(exe: &Path, icon: &Path, class: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Koma\n\
         Comment=Agentic coding desktop client\n\
         Exec={exec}\n\
         Icon={icon}\n\
         Terminal=false\n\
         Categories=Development;\n\
         StartupNotify=true\n\
         StartupWMClass={class}\n\
         Keywords=ai;agent;coding;\n",
        exec = desktop_exec(exe),
        icon = icon.display(),
    )
}

fn desktop_exec(exe: &Path) -> String {
    let raw = exe.display().to_string();
    if raw.contains([' ', '\t', '\n', '"', '\\']) {
        let escaped = raw.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\" gui")
    } else {
        format!("{raw} gui")
    }
}

fn install_macos(home: &Path, exe: &Path, register: bool) -> Result<Installed> {
    let app = home.join("Applications/Koma.app");
    let macos = app.join("Contents/MacOS");
    let resources = app.join("Contents/Resources");
    fs::create_dir_all(&macos)?;
    fs::create_dir_all(&resources)?;
    write_bytes(&resources.join("AppIcon.icns"), ICON_ICNS)?;
    let png = resources.join("icon-256.png");
    write_bytes(&png, ICON_256)?;
    let bundled = macos.join("koma");
    place_bundle_executable(&bundled, exe)?;
    let plist = macos_plist();
    write_bytes(&app.join("Contents/Info.plist"), plist.as_bytes())?;
    if register {
        register_macos(&app);
    }
    Ok(Installed { location: app })
}

fn macos_plist() -> String {
    "\
<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
<dict>
  <key>CFBundleName</key>
  <string>Koma</string>
  <key>CFBundleDisplayName</key>
  <string>Koma</string>
  <key>CFBundleIdentifier</key>
  <string>run.koma.desktop</string>
  <key>CFBundleVersion</key>
  <string>1.0</string>
  <key>CFBundleShortVersionString</key>
  <string>1.0</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleExecutable</key>
  <string>koma</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSPrincipalClass</key>
  <string>NSApplication</string>
</dict>
</plist>
"
    .to_string()
}

/// Copy `exe` into the bundle. Skip when the running image already is that
/// file, and replace it via a temp name so a running bundle executable is not
/// truncated in place.
fn place_bundle_executable(dest: &Path, exe: &Path) -> Result<()> {
    if let (Ok(src), Ok(current)) = (exe.canonicalize(), dest.canonicalize()) {
        if src == current {
            return Ok(());
        }
    }
    if bundle_exe_current(exe, dest) {
        return Ok(());
    }
    if dest.exists() {
        // An older shell trampoline is not the Mach-O Launchpad expects.
        let _ = fs::remove_file(dest);
    }
    let tmp = dest.with_file_name("koma.new");
    fs::copy(exe, &tmp).with_context(|| format!("copy {} into the app bundle", exe.display()))?;
    set_executable(&tmp)?;
    fs::rename(&tmp, dest).with_context(|| format!("install {}", dest.display()))?;
    Ok(())
}

fn bundle_exe_current(src: &Path, dest: &Path) -> bool {
    let (Ok(src_meta), Ok(dest_meta)) = (fs::metadata(src), fs::metadata(dest)) else {
        return false;
    };
    if !dest_meta.is_file() || src_meta.len() != dest_meta.len() || src_meta.len() == 0 {
        return false;
    }
    // Same length is not proof, but a shell trampoline is a few dozen bytes
    // and the koma binary is not. A matching length means we already copied.
    dest_meta.len() > 4096
}

fn is_app_bundle_exe(exe: &Path) -> bool {
    let mut saw_app = false;
    let mut saw_contents = false;
    let mut saw_macos = false;
    for component in exe.components() {
        let name = component.as_os_str().to_string_lossy();
        if name.ends_with(".app") {
            saw_app = true;
            saw_contents = false;
            saw_macos = false;
        } else if saw_app && name == "Contents" {
            saw_contents = true;
        } else if saw_contents && name == "MacOS" {
            saw_macos = true;
        }
    }
    saw_app && saw_macos
}

/// Rebuild the running desktop's app index. Same idea as copying an app out
/// of a DMG: touch the bundle, force Launch Services to rescan it, and import
/// it into Spotlight. No Dock restart and no logout.
fn register_macos(app: &Path) {
    let _ = Command::new("xattr")
        .args(["-dr", "com.apple.quarantine"])
        .arg(app)
        .status();
    let _ = Command::new("touch").arg(app).status();
    let lsregister = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    let _ = Command::new(lsregister)
        .args(["-f", "-R", "-trusted"])
        .arg(app)
        .status();
    let _ = Command::new("mdimport").arg(app).status();
}

/// Tell the running session the new desktop file and icon are there. GNOME and
/// KDE watch the applications directory; these commands rebuild the caches a
/// file-manager install would refresh.
fn register_linux(apps: &Path, icons: &Path, desktop: &Path) {
    let _ = Command::new("touch").arg(desktop).status();
    let _ = Command::new("update-desktop-database").arg(apps).status();
    let _ = Command::new("gtk-update-icon-cache")
        .args(["-f", "-t"])
        .arg(icons)
        .status();
    let _ = Command::new("xdg-desktop-menu").arg("forceupdate").status();
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(path, bytes).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn set_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Decode the embedded 256px icon for the Linux window / taskbar icon.
#[cfg(all(feature = "gui", target_os = "linux"))]
pub fn window_icon() -> Option<tao::window::Icon> {
    let decoder = png::Decoder::new(std::io::Cursor::new(ICON_256));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let pixels = buf[..info.buffer_size()].to_vec();
    tao::window::Icon::from_rgba(pixels, info.width, info.height).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_with_no_args_opens_the_gui() {
        let exe = Path::new("/Users/a/Applications/Koma.app/Contents/MacOS/koma");
        let args = vec!["/Users/a/Applications/Koma.app/Contents/MacOS/koma".to_string()];
        assert!(exe_should_launch_gui(exe, &args));
        let launched = vec![args[0].clone(), "-psn_0_123".to_string()];
        assert!(exe_should_launch_gui(exe, &launched));
    }

    #[test]
    fn bundle_with_a_subcommand_stays_a_cli() {
        let exe = Path::new("/Users/a/Applications/Koma.app/Contents/MacOS/koma");
        let args = vec![exe.display().to_string(), "--daemon".to_string()];
        assert!(!exe_should_launch_gui(exe, &args));
        assert!(!exe_should_launch_gui(
            Path::new("/usr/local/bin/koma"),
            &args
        ));
    }

    #[test]
    fn wrapper_layout_uses_the_real_window_class() {
        let root = std::env::temp_dir().join(format!(
            "koma-wm-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let wrapper = root.join("koma");
        let bin = root.join("koma.bin");
        fs::write(&wrapper, b"#!/bin/sh\n").unwrap();
        fs::write(&bin, b"elf").unwrap();
        assert_eq!(wm_class(&wrapper), "koma.bin");
        assert_eq!(linux_launch_exe(&bin), wrapper);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn deb_layout_uses_koma_as_the_window_class() {
        let root = std::env::temp_dir().join(format!("koma-wm-deb-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let exe = root.join("koma");
        fs::write(&exe, b"elf").unwrap();
        assert_eq!(wm_class(&exe), "koma");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn linux_install_writes_desktop_entry_and_icon() {
        let root = std::env::temp_dir().join(format!(
            "koma-desktop-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        let bin_dir = root.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let exe = bin_dir.join("koma");
        fs::write(&exe, b"#!/bin/sh\n").unwrap();
        let installed = install_linux(&home, &home.join(".local/share"), &exe, false).unwrap();
        let desktop = fs::read_to_string(&installed.location).unwrap();
        let icon = home.join(".local/share/icons/hicolor/256x256/apps/koma.png");
        assert!(desktop.contains("Name=Koma\n"));
        assert!(desktop.contains(&format!("Exec={} gui\n", exe.display())));
        assert!(desktop.contains(&format!("Icon={}\n", icon.display())));
        assert!(desktop.contains("StartupWMClass=koma\n"));
        assert_eq!(fs::read(&icon).unwrap(), ICON_256);
        assert_eq!(
            fs::read(home.join(".local/share/icons/hicolor/48x48/apps/koma.png")).unwrap(),
            ICON_48
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn macos_install_writes_bundle_with_icon_and_binary() {
        let root = std::env::temp_dir().join(format!(
            "koma-app-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let home = root.join("home");
        let exe = root.join("koma");
        fs::write(&exe, vec![0u8; 8192]).unwrap();
        let installed = install_macos(&home, &exe, false).unwrap();
        assert_eq!(installed.location, home.join("Applications/Koma.app"));
        let plist = fs::read_to_string(installed.location.join("Contents/Info.plist")).unwrap();
        assert!(plist.contains("<string>Koma</string>"));
        assert!(plist.contains("<string>AppIcon</string>"));
        assert!(plist.contains("<string>koma</string>"));
        assert_eq!(
            fs::read(installed.location.join("Contents/Resources/AppIcon.icns")).unwrap(),
            ICON_ICNS
        );
        assert_eq!(
            fs::read(installed.location.join("Contents/MacOS/koma")).unwrap(),
            vec![0u8; 8192]
        );
        let _ = fs::remove_dir_all(&root);
    }
}
