//! Compiling C model sources into shared libraries.
//!
//! This file is shared verbatim with `build.rs` (via `#[path]`), so the presets
//! shipped with the app are built by exactly the code that builds user plugins
//! at runtime. It must therefore depend on nothing but `std` and `cc`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Bump when the plugin ABI changes so stale cached libraries are rebuilt.
pub const ABI_TAG: &str = "fit_it-abi-2";

pub fn is_c_source(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "c")
}

pub fn lib_extension(target: &str) -> &'static str {
    if target.contains("windows") {
        "dll"
    } else if target.contains("apple") {
        "dylib"
    } else {
        "so"
    }
}

fn fnv1a(bytes: &[u8], mut h: u64) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// `<build_dir>/<stem>-<hash>.<ext>`, where the hash covers the source text, the
/// target and the ABI version. A content hash (not mtime) keeps prebuilt preset
/// libraries valid after they are copied by an installer.
pub fn output_path(src: &Path, build_dir: &Path, target: &str) -> Result<PathBuf, String> {
    let text = std::fs::read(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let mut h = fnv1a(&text, 0xcbf29ce484222325);
    h = fnv1a(target.as_bytes(), h);
    h = fnv1a(ABI_TAG.as_bytes(), h);
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("model");
    Ok(build_dir.join(format!("{stem}-{h:016x}.{}", lib_extension(target))))
}

fn run(mut cmd: Command, what: &str) -> Result<String, String> {
    let out = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("{what} not found — install it or add it to PATH")
        } else {
            format!("could not run {what}: {e}")
        }
    })?;
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        Ok(log)
    } else {
        Err(format!("{what} failed:\n{}", log.trim()))
    }
}

/// Compile `src` into the shared library `out`. Returns the compiler log.
pub fn compile(src: &Path, out: &Path, target: &str) -> Result<String, String> {
    let build_dir = out.parent().ok_or("bad output path")?;
    // Compilers leave import libraries, object and debug files next to their
    // output; build in a scratch directory and keep only the library.
    let scratch = build_dir.join(format!(
        "tmp-{}",
        out.file_stem().and_then(|s| s.to_str()).unwrap_or("build")
    ));
    std::fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let tmp_out = scratch.join(out.file_name().ok_or("bad output path")?);
    let include = src.parent().unwrap_or(Path::new("."));

    let tool = cc::Build::new()
        .target(target)
        .host(target)
        .opt_level(2)
        .debug(false)
        .cargo_metadata(false)
        .cargo_warnings(false)
        .try_get_compiler()
        .map_err(|e| format!("no C compiler found: {e}"))?;
    let mut cmd = tool.to_command();
    if tool.is_like_msvc() {
        cmd.arg("/LD")
            .arg(format!("/I{}", include.display()))
            .arg(format!("/Fe{}", tmp_out.display()))
            .arg(format!("/Fo{}\\", scratch.display()))
            .arg(src);
    } else {
        cmd.arg("-shared")
            .arg("-I")
            .arg(include)
            .arg("-o")
            .arg(&tmp_out)
            .arg(src);
        if !target.contains("windows") {
            cmd.arg("-fPIC").arg("-lm");
        }
    }
    let result = run(cmd, &format!("C compiler ({})", tool.path().display()));
    let result = result.and_then(|log| {
        std::fs::rename(&tmp_out, out)
            .or_else(|_| std::fs::copy(&tmp_out, out).map(|_| ()))
            .map_err(|e| format!("{}: {e}", out.display()))?;
        Ok(log)
    });
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

/// Return the cached library for `src`, compiling it first if needed.
/// The bool is true when a compile actually happened.
pub fn build_cached(src: &Path, build_dir: &Path, target: &str) -> Result<(PathBuf, bool), String> {
    if !is_c_source(src) {
        return Err("not a C source file".into());
    }
    let out = output_path(src, build_dir, target)?;
    if out.exists() {
        return Ok((out, false));
    }
    std::fs::create_dir_all(build_dir).map_err(|e| format!("{}: {e}", build_dir.display()))?;
    compile(src, &out, target)?;
    // Remove libraries built from older versions of this source. They may still
    // be loaded (and locked on Windows); those are cleaned up on a later run.
    if let (Some(stem), Ok(dir)) = (
        src.file_stem().and_then(|s| s.to_str()),
        std::fs::read_dir(build_dir),
    ) {
        let prefix = format!("{stem}-");
        for e in dir.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let rest = name.strip_prefix(&prefix).unwrap_or("");
            // `<stem>-<16 hex>.<ext>` only, so `foo` never deletes `foo-bar`'s output.
            if p != out
                && rest.len() > 17
                && rest.as_bytes()[16] == b'.'
                && rest[..16].bytes().all(|b| b.is_ascii_hexdigit())
            {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    Ok((out, true))
}
