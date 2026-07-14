use std::process::Command;

fn main() {
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
