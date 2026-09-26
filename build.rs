// The preset models are compiled by the very module the app uses for user
// plugins, so presets and plugins cannot drift apart.
#[path = "src/plugin/compile.rs"]
#[allow(dead_code)]
mod compile;

use std::path::{Path, PathBuf};

fn main() {
    // Embed the icon into the .exe so Explorer and the taskbar show it even
    // before the app creates a window.
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("resources/icon.ico");
        res.compile().unwrap();
    }
    println!("cargo:rerun-if-changed=resources/icon.ico");

    let target = std::env::var("TARGET").unwrap();
    println!("cargo:rustc-env=FIT_IT_TARGET={target}");

    build_presets(&target);
}

/// Copy `presets/` next to the binary (target/<profile>/presets) and prebuild its
/// C sources into `presets/.build`, exactly where the app's loader looks.
fn build_presets(target: &str) {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let src_dir = manifest.join("presets");
    let header = manifest.join("resources/plugin_templates/fit_it_plugin.h");
    println!("cargo:rerun-if-changed=presets");
    println!("cargo:rerun-if-changed={}", header.display());
    println!("cargo:rerun-if-changed=src/plugin/compile.rs");

    // OUT_DIR = target/[<triple>/]<profile>/build/<pkg>-<hash>/out
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let profile_dir = out_dir.ancestors().nth(3).unwrap().to_path_buf();
    let dest = profile_dir.join("presets");
    std::fs::create_dir_all(&dest).unwrap();
    copy_if_changed(&header, &dest.join("fit_it_plugin.h"));

    let mut entries: Vec<_> = std::fs::read_dir(&src_dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for path in entries.into_iter().filter(|p| p.is_file()) {
        let to = dest.join(path.file_name().unwrap());
        copy_if_changed(&path, &to);
        if compile::is_c_source(&to) {
            match compile::build_cached(&to, &dest.join(".build"), target) {
                Ok(_) => {}
                // A missing compiler should not block building the app itself; the
                // presets then get compiled on first launch if a compiler shows up.
                Err(e) if e.starts_with("no C compiler") => {
                    println!("cargo:warning=preset {} not prebuilt: {e}", path.display());
                }
                Err(e) => panic!("preset {} failed to compile:\n{e}", path.display()),
            }
        }
    }
}

fn copy_if_changed(from: &Path, to: &Path) {
    let new = std::fs::read(from).unwrap();
    if std::fs::read(to).ok().as_deref() != Some(new.as_slice()) {
        std::fs::write(to, new).unwrap();
    }
}
