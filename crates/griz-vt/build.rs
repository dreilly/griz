use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=src/ghostty_shim.c");
    println!("cargo:rerun-if-env-changed=GHOSTTY_VT_DIR");

    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS is set by Cargo");
    let target_arch =
        env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH is set by Cargo");
    if target_os != "macos" {
        panic!(
            "griz-vt currently supports only macOS using the official libghostty-vt 1.0.0 XCFramework; target OS is {target_os}"
        );
    }

    let lipo_arch = match target_arch.as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!(
            "griz-vt supports only arm64 and x86_64 macOS targets; target architecture is {other}"
        ),
    };

    let root = env::var_os("GHOSTTY_VT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(default_ghostty_root);
    let (include_dir, library) = locate_ghostty(&root).unwrap_or_else(|| {
        panic!(
            "libghostty-vt 1.0.0 was not found under {}. Run scripts/setup-ghostty.sh, or set GHOSTTY_VT_DIR to the extracted ghostty-vt.xcframework, its macOS slice, or an installed prefix",
            root.display()
        )
    });

    check_architecture(&library, lipo_arch);

    cc::Build::new()
        .file("src/ghostty_shim.c")
        .include(&include_dir)
        .define("GHOSTTY_STATIC", None)
        .flag_if_supported("-std=c11")
        .warnings(true)
        .compile("griz_vt_shim");

    let library_dir = library
        .parent()
        .expect("libghostty-vt library path has a parent");
    println!("cargo:rustc-link-search=native={}", library_dir.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt");
}

fn default_ghostty_root() -> PathBuf {
    PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by Cargo"))
        .join("../../vendor/ghostty-vt-1.0.0.xcframework")
}

fn locate_ghostty(root: &Path) -> Option<(PathBuf, PathBuf)> {
    let candidates = [
        root.join("ghostty-vt.xcframework/macos-arm64_x86_64"),
        root.join("macos-arm64_x86_64"),
        root.to_path_buf(),
    ];

    for slice in candidates {
        let headers = slice.join("Headers");
        let library = slice.join("libghostty-vt.a");
        if headers.join("ghostty/vt.h").is_file() && library.is_file() {
            return Some((headers, library));
        }
    }

    let headers = root.join("include");
    let library = root.join("lib/libghostty-vt.a");
    if headers.join("ghostty/vt.h").is_file() && library.is_file() {
        return Some((headers, library));
    }

    None
}

fn check_architecture(library: &Path, required_arch: &str) {
    let output = Command::new("lipo")
        .arg("-archs")
        .arg(library)
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "failed to run lipo while checking {}: {error}. Install the Xcode command-line tools",
                library.display()
            )
        });
    if !output.status.success() {
        panic!(
            "lipo could not inspect {}: {}",
            library.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let archs = String::from_utf8_lossy(&output.stdout);
    if !archs.split_whitespace().any(|arch| arch == required_arch) {
        panic!(
            "{} does not contain the required macOS architecture {required_arch} (found: {})",
            library.display(),
            archs.trim()
        );
    }
}
