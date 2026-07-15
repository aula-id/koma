use std::process::Command;

fn main() {
    embed_windows_resource();

    // Only build the web UI when the gui feature is enabled.
    if std::env::var("CARGO_FEATURE_GUI").is_err() {
        return;
    }
    let webgui = concat!(env!("CARGO_MANIFEST_DIR"), "/../src-webgui");
    // Rebuild if any frontend source changes.
    println!("cargo:rerun-if-changed={webgui}/src");
    println!("cargo:rerun-if-changed={webgui}/index.html");
    println!("cargo:rerun-if-changed={webgui}/package.json");
    println!("cargo:rerun-if-changed={webgui}/vite.config.ts");

    // npm must be on PATH (run `nvm use 24` before cargo build). Fail loudly if not.
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let install = Command::new(npm).arg("install").current_dir(webgui).status();
    match install {
        Ok(s) if s.success() => {}
        Ok(s) => panic!("`npm install` in src-webgui failed with status {s}. Run `nvm use 24` first."),
        Err(e) => panic!("could not run `npm` ({e}). The `gui` feature needs Node/npm on PATH — run `nvm use 24` before building, or build without --features gui."),
    }
    let build = Command::new(npm).args(["run", "build"]).current_dir(webgui).status()
        .expect("failed to spawn npm run build");
    if !build.success() { panic!("`npm run build` (vite) failed"); }
}

// Embeds koma.exe's PE resources (icon + version metadata) so Explorer shows
// the koma icon instead of a generic placeholder on the raw exe. Applies to
// ALL windows builds (not gui-feature-gated) — the TUI exe deserves the icon
// too, unlike the npm build step above which only concerns the GUI's web assets.
fn embed_windows_resource() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let icon_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/icon.ico");
    println!("cargo:rerun-if-changed={icon_path}");

    let version = env!("CARGO_PKG_VERSION");
    let mut res = winresource::WindowsResource::new();
    res.set_icon(icon_path);
    res.set("ProductName", "koma");
    res.set("FileDescription", "koma - AI coding agent");
    res.set("ProductVersion", version);

    // winresource::compile() always returns a Result — it never panics
    // internally, whether the failure is a missing rc.exe/windres toolchain
    // or a real compile error (see BenjaminRi/winresource lib.rs:
    // compile_with_toolkit_{msvc,gnu} both propagate `process::Command`
    // spawn/exit failures as `io::Error`s rather than aborting).
    //
    // We only get to choose panic-vs-warn here. Native Windows builds (CI
    // runners + user machines building on Windows itself) always have rc.exe
    // via the MSVC toolchain, or windres via MinGW when using the GNU
    // toolchain — a failure there means something is genuinely broken, so we
    // hard-error. Cross-compiling *from* a non-Windows host (e.g. a Linux dev
    // box building --target x86_64-pc-windows-gnu) is a much softer
    // environment: windres/mingw-w64 may simply not be installed, and losing
    // the embedded icon isn't worth failing the whole build over — warn and
    // move on so `cargo build/check` still succeeds.
    if let Err(e) = res.compile() {
        let is_native_windows_host = cfg!(target_os = "windows");
        if is_native_windows_host {
            panic!(
                "failed to embed the Windows exe icon/resource (rc.exe should be available via the MSVC toolchain, or windres via MinGW): {e}"
            );
        } else {
            println!(
                "cargo:warning=skipping Windows exe icon/resource embed while cross-compiling (resource compiler likely missing, e.g. windres/mingw-w64 not installed): {e}"
            );
        }
    }
}
