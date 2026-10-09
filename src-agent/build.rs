use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../src-misc/help");
    embed_windows_resource();

    // Only build the web UI when the gui feature is enabled.
    if std::env::var("CARGO_FEATURE_GUI").is_err() {
        return;
    }
    #[cfg(feature = "gui")]
    build_computer_bridge();
    let webgui = concat!(env!("CARGO_MANIFEST_DIR"), "/../src-webgui");
    // Rebuild if any frontend source changes.
    println!("cargo:rerun-if-changed={webgui}/src");
    println!("cargo:rerun-if-changed={webgui}/index.html");
    println!("cargo:rerun-if-changed={webgui}/package.json");
    println!("cargo:rerun-if-changed={webgui}/vite.config.ts");

    // npm must be on PATH (run `nvm use 24` before cargo build). Fail loudly if not.
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let install = Command::new(npm)
        .arg("install")
        .current_dir(webgui)
        .status();
    match install {
        Ok(s) if s.success() => {}
        Ok(s) => panic!("`npm install` in src-webgui failed with status {s}. Run `nvm use 24` first."),
        Err(e) => panic!("could not run `npm` ({e}). The `gui` feature needs Node/npm on PATH — run `nvm use 24` before building, or build without --features gui."),
    }
    let build = Command::new(npm)
        .args(["run", "build"])
        .current_dir(webgui)
        .status()
        .expect("failed to spawn npm run build");
    if !build.success() {
        panic!("`npm run build` (vite) failed");
    }
}

#[cfg(feature = "gui")]
fn build_computer_bridge() {
    println!("cargo:rerun-if-changed=native/computer/capture_limits.h");
    println!("cargo:rerun-if-changed=native/computer/input_idle.h");
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let mut build = cc::Build::new();
    // cc-rs uses Cargo's TARGET for the archive architecture (Darwin arm64 or
    // x86_64), and the shared MACOSX_DEPLOYMENT_TARGET for its OS baseline.
    // Its rustc-link-lib output belongs to src/lib.rs; the executable imports
    // that library's computer_native ABI so these dependencies reach the link.
    build.cpp(true);
    match target.as_str() {
        "macos" => {
            let source = "native/computer/macos.mm";
            println!("cargo:rerun-if-changed={source}");
            build
                .file(source)
                .flag("-std=c++17")
                .flag("-fobjc-arc")
                .flag("-fblocks");
            build.compile("koma_computer");
            for framework in [
                "Foundation",
                "AppKit",
                "ApplicationServices",
                "Vision",
                "ImageIO",
            ] {
                println!("cargo:rustc-link-lib=framework={framework}");
            }
            // Older macOS releases can still run Koma; the bridge advertises
            // capture only on macOS 14+, where the screenshot API exists.
            println!("cargo:rustc-link-arg=-Wl,-weak_framework,ScreenCaptureKit");
        }
        "windows" => {
            let source = "native/computer/windows.cpp";
            println!("cargo:rerun-if-changed={source}");
            assert!(std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc"),
                "The GUI computer bridge requires the Windows MSVC SDK; use --no-default-features for GNU headless builds");
            build
                .file(source)
                // VS 18 rejects <experimental/coroutine>, which C++/WinRT includes
                // when this file is built as C++17. C++20 selects <coroutine>.
                .flag("/std:c++20")
                .flag("/EHsc")
                .flag("/utf-8");
            build
                .define("WIN32_LEAN_AND_MEAN", None)
                .define("NOMINMAX", None)
                .define("_SILENCE_EXPERIMENTAL_COROUTINE_DEPRECATION_WARNINGS", None);
            build.compile("koma_computer");
            for library in [
                "windowsapp",
                "d3d11",
                "dxgi",
                "dwmapi",
                "user32",
                "ole32",
                "oleaut32",
                "uuid",
                "uiautomationcore",
                "windowscodecs",
            ] {
                println!("cargo:rustc-link-lib={library}");
            }
        }
        _ => {}
    }
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
