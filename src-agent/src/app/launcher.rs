//! App-list entry and icon for Linux and macOS.
//!
//! `koma update` and `install.sh` both end in `koma launcher-install`. The icon
//! bytes are compiled into the binary, so placement does not depend on a second
//! download. Linux writes a user desktop file plus hicolor PNGs. macOS writes
//! `~/Applications/Koma.app` whose executable is this binary, so Launchpad and
//! the Dock use `AppIcon.icns`.

use std::fs;
use std::io::Read;
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
///
/// Replaces the bundle executable when it is not the image this process is
/// running from. `koma update` and `koma launcher-install` use this.
pub fn install() -> Result<Installed> {
    let home = dirs::home_dir().context("HOME is not set")?;
    let exe = installed_exe().context("could not find the koma binary to launch")?;
    install_at(&home, &exe, true, true)
}

/// Refresh the app-list entry while a GUI is opening.
///
/// On macOS, prefer `~/.local/bin/koma` (the curl installer / `koma update`
/// target) so the app bundle picks up a new CLI binary. That path is not under
/// Documents / Desktop / Downloads, so stating it does not raise a folder
/// dialog. Copy is skipped while this process is the bundle image. A tree
/// under a protected folder is never used as the source.
#[cfg(feature = "gui")]
pub fn install_for_launch() -> Result<Installed> {
    let home = dirs::home_dir().context("HOME is not set")?;
    #[cfg(target_os = "macos")]
    {
        if let Some(cli) = installed_release_exe() {
            if !in_protected_folder(&cli) {
                return install_macos(&home, &cli, true, true);
            }
        }
        let bundled = home.join("Applications/Koma.app/Contents/MacOS/koma");
        if let Ok(meta) = fs::metadata(&bundled) {
            if meta.is_file() && meta.len() > 4096 {
                return install_macos(&home, &bundled, true, false);
            }
        }
    }
    let exe = installed_exe().context("could not find the koma binary to launch")?;
    install_at(&home, &exe, true, false)
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

#[cfg(any(target_os = "macos", test))]
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

fn install_at(home: &Path, exe: &Path, register: bool, replace_binary: bool) -> Result<Installed> {
    // `if cfg!` (not `#[cfg]`) so a Linux build still typechecks the macOS
    // bundle path. `cargo check` denies dead code.
    if cfg!(target_os = "macos") {
        install_macos(home, exe, register, replace_binary)
    } else if cfg!(target_os = "linux") {
        let _ = replace_binary;
        install_linux(home, &data_home(home), exe, register)
    } else {
        let _ = replace_binary;
        anyhow::bail!("app list placement supports Linux and macOS")
    }
}

fn installed_exe() -> Option<PathBuf> {
    installed_release_exe().or_else(|| {
        std::env::current_exe()
            .ok()
            .filter(|path| path.is_file())
            .map(|path| linux_launch_exe(&path))
    })
}

/// The curl / `koma update` install location, not a checkout or the app bundle.
fn installed_release_exe() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::var_os("KOMA_INSTALL_DIR") {
        candidates.push(PathBuf::from(dir).join("koma"));
    }
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local/bin/koma"));
        candidates.push(home.join(".local/bin/koma.bin"));
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
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
    let mut changed = false;
    for (size, bytes) in LINUX_SIZES {
        let dest = icons.join(format!("{size}x{size}/apps/koma.png"));
        changed |= write_bytes(&dest, bytes)?;
        if *size == 256 {
            icon_file = Some(dest);
        }
    }
    let icon_file = icon_file.context("256px icon was not written")?;
    let apps = data_home.join("applications");
    let desktop = apps.join("koma.desktop");
    let body = linux_desktop(exe, &icon_file, wm_class(exe));
    changed |= write_bytes(&desktop, body.as_bytes())?;
    if register && changed {
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

fn install_macos(
    home: &Path,
    exe: &Path,
    register: bool,
    replace_binary: bool,
) -> Result<Installed> {
    let app = home.join("Applications/Koma.app");
    let macos = app.join("Contents/MacOS");
    let resources = app.join("Contents/Resources");
    fs::create_dir_all(&macos)?;
    fs::create_dir_all(&resources)?;
    let mut changed = false;
    changed |= write_bytes(&resources.join("AppIcon.icns"), ICON_ICNS)?;
    let png = resources.join("icon-256.png");
    changed |= write_bytes(&png, ICON_256)?;
    let bundled = macos.join("koma");
    changed |= place_bundle_executable(&bundled, exe, replace_binary)?;
    let plist = macos_plist();
    changed |= write_bytes(&app.join("Contents/Info.plist"), plist.as_bytes())?;
    // lsregister on every open changes the ad-hoc identity, so a folder grant
    // does not stick and macOS asks again.
    if register && changed {
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

/// Copy `exe` into the bundle. Returns whether the bundle executable changed.
///
/// A GUI open passes `replace = false` so an existing binary is left in place
/// and `exe` is not stat'd. That stat is the macOS Documents / Desktop /
/// Downloads prompt when the checkout lives in one of those folders. `koma
/// update` passes `replace = true`. The running image is never replaced:
/// overwriting the mapped Mach-O kills the process, and the next open asks
/// for the folder again.
fn place_bundle_executable(dest: &Path, exe: &Path, replace: bool) -> Result<bool> {
    if running_image_is(dest) {
        return Ok(false);
    }
    if !replace {
        if let Ok(meta) = fs::metadata(dest) {
            if meta.is_file() && meta.len() > 4096 {
                return Ok(false);
            }
        }
    }
    if let (Ok(src), Ok(current)) = (exe.canonicalize(), dest.canonicalize()) {
        if src == current {
            return Ok(false);
        }
    }
    if files_have_same_bytes(exe, dest) {
        return Ok(false);
    }
    if dest.exists() {
        // An older shell trampoline is not the Mach-O Launchpad expects.
        let _ = fs::remove_file(dest);
    }
    let tmp = dest.with_file_name("koma.new");
    fs::copy(exe, &tmp).with_context(|| format!("copy {} into the app bundle", exe.display()))?;
    set_executable(&tmp)?;
    fs::rename(&tmp, dest).with_context(|| format!("install {}", dest.display()))?;
    Ok(true)
}

/// True when `dest` is the Mach-O this process is mapped from.
///
/// Canonicalize the destination only. Canonicalizing `current_exe` prompts
/// when that path is under Documents, Desktop, or Downloads.
fn running_image_is(dest: &Path) -> bool {
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    if current == dest {
        return true;
    }
    let Ok(placed) = dest.canonicalize() else {
        return false;
    };
    if current == placed {
        return true;
    }
    if in_protected_folder(&current) {
        return false;
    }
    std::fs::canonicalize(&current).ok().as_deref() == Some(placed.as_path())
}

fn in_protected_folder(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some("Desktop" | "Documents" | "Downloads")
        )
    })
}

/// True when `dest` already holds the same bytes as `src`.
///
/// Length alone is not identity: consecutive koma releases are often the same
/// size, and treating that as "already copied" left `Koma.app` on the old
/// binary after `koma update`.
fn files_have_same_bytes(src: &Path, dest: &Path) -> bool {
    let (Ok(src_meta), Ok(dest_meta)) = (fs::metadata(src), fs::metadata(dest)) else {
        return false;
    };
    if !dest_meta.is_file() || src_meta.len() != dest_meta.len() || src_meta.len() == 0 {
        return false;
    }
    let Ok(mut fa) = fs::File::open(src) else {
        return false;
    };
    let Ok(mut fb) = fs::File::open(dest) else {
        return false;
    };
    let mut ba = [0u8; 65536];
    let mut bb = [0u8; 65536];
    loop {
        let na = match fa.read(&mut ba) {
            Ok(n) => n,
            Err(_) => return false,
        };
        let nb = match fb.read(&mut bb) {
            Ok(n) => n,
            Err(_) => return false,
        };
        if na != nb || ba[..na] != bb[..nb] {
            return false;
        }
        if na == 0 {
            return true;
        }
    }
}

#[cfg(any(target_os = "macos", test))]
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

/// Write `bytes` when the file is missing or different. `false` means the
/// bytes were already there, so the caller can skip Launch Services.
fn write_bytes(path: &Path, bytes: &[u8]) -> Result<bool> {
    if let Ok(existing) = fs::read(path) {
        if existing == bytes {
            return Ok(false);
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(path, bytes).with_context(|| format!("write {}", path.display()))?;
    Ok(true)
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
    fn desktop_exec_escapes_backslashes() {
        assert_eq!(
            desktop_exec(Path::new(r"C:\Users\a\koma")),
            "\"C:\\\\Users\\\\a\\\\koma\" gui"
        );
        assert_eq!(
            desktop_exec(Path::new("/usr/local/bin/koma")),
            "/usr/local/bin/koma gui"
        );
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
        // Windows temp paths contain `\`, which a desktop Exec line must quote
        // and escape. `desktop_exec` is that line, including the `gui` argument.
        let exec = desktop_exec(&exe);
        assert!(exec.ends_with(" gui"));
        assert!(desktop.contains(&format!("Exec={exec}\n")));
        // Path::join keeps each segment's slash characters. On Windows the
        // Icon line and one long join name the same file but do not match as text.
        let written_icon = desktop
            .lines()
            .find_map(|line| line.strip_prefix("Icon="))
            .expect("desktop entry has an Icon line");
        assert_eq!(Path::new(written_icon), icon.as_path());
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
        let installed = install_macos(&home, &exe, false, true).unwrap();
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

    #[test]
    fn write_bytes_leaves_matching_files_untouched() {
        let root = std::env::temp_dir().join(format!(
            "koma-bytes-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&root);
        let path = root.join("nested/icon.png");
        assert!(write_bytes(&path, b"png").unwrap());
        assert!(!write_bytes(&path, b"png").unwrap());
        assert_eq!(fs::read(&path).unwrap(), b"png");
        assert!(write_bytes(&path, b"png2").unwrap());
        assert_eq!(fs::read(&path).unwrap(), b"png2");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn macos_launch_keeps_a_real_bundle_binary_when_the_source_is_missing() {
        let root = std::env::temp_dir().join(format!(
            "koma-app-keep-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let home = root.join("home");
        let source = root.join("koma");
        fs::write(&source, vec![7u8; 8192]).unwrap();
        install_macos(&home, &source, false, true).unwrap();
        fs::remove_file(&source).unwrap();
        install_macos(&home, &source, false, false).unwrap();
        assert_eq!(
            fs::read(home.join("Applications/Koma.app/Contents/MacOS/koma")).unwrap(),
            vec![7u8; 8192]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn macos_launch_does_not_replace_an_existing_bundle_binary() {
        let root = std::env::temp_dir().join(format!(
            "koma-app-norepl-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let home = root.join("home");
        let source = root.join("koma");
        fs::write(&source, vec![0u8; 8192]).unwrap();
        install_macos(&home, &source, false, true).unwrap();
        let other = root.join("other");
        fs::write(&other, vec![9u8; 9000]).unwrap();
        install_macos(&home, &other, false, false).unwrap();
        assert_eq!(
            fs::read(home.join("Applications/Koma.app/Contents/MacOS/koma")).unwrap(),
            vec![0u8; 8192]
        );
        // Same length as the installed binary, different bytes: replace=true
        // must copy. Length-only identity left Koma.app stale after koma update.
        let newer = root.join("newer");
        fs::write(&newer, vec![3u8; 8192]).unwrap();
        install_macos(&home, &newer, false, true).unwrap();
        assert_eq!(
            fs::read(home.join("Applications/Koma.app/Contents/MacOS/koma")).unwrap(),
            vec![3u8; 8192]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn macos_launch_replaces_a_shell_trampoline() {
        let root = std::env::temp_dir().join(format!(
            "koma-app-tramp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let home = root.join("home");
        let source = root.join("koma");
        fs::write(&source, vec![0u8; 8192]).unwrap();
        install_macos(&home, &source, false, true).unwrap();
        let bundled = home.join("Applications/Koma.app/Contents/MacOS/koma");
        fs::write(&bundled, b"#!/bin/sh\nexec koma gui\n").unwrap();
        let fresh = root.join("fresh");
        fs::write(&fresh, vec![3u8; 8192]).unwrap();
        install_macos(&home, &fresh, false, false).unwrap();
        assert_eq!(fs::read(&bundled).unwrap(), vec![3u8; 8192]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn protected_folder_is_documents_desktop_or_downloads() {
        assert!(in_protected_folder(Path::new(
            "/Users/a/Documents/koma/target/release/koma"
        )));
        assert!(in_protected_folder(Path::new("/Users/a/Desktop/koma")));
        assert!(in_protected_folder(Path::new("/Users/a/Downloads/koma")));
        assert!(!in_protected_folder(Path::new(
            "/Users/a/Applications/Koma.app/Contents/MacOS/koma"
        )));
        assert!(!in_protected_folder(Path::new("/Users/a/.local/bin/koma")));
        assert!(!in_protected_folder(Path::new(
            "/Users/a/Projects/koma/target/release/koma"
        )));
    }

    #[test]
    fn running_image_is_this_process() {
        let exe = std::env::current_exe().unwrap();
        assert!(running_image_is(&exe));
        assert!(!running_image_is(Path::new("/no/such/koma-binary")));
    }
}
