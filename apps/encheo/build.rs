fn main() {
    slint_build::compile("ui/app.slint").expect("compile ui");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        return link_ios();
    }
    // libmpv: ask pkg-config where it lives (Homebrew, distro packages). Release builds bundle
    // their own libmpv and set MPV_LIB_DIR instead.
    println!("cargo:rerun-if-env-changed=MPV_LIB_DIR");
    if let Ok(dir) = std::env::var("MPV_LIB_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
    } else if let Ok(out) = std::process::Command::new("pkg-config").args(["--libs-only-L", "mpv"]).output() {
        for flag in String::from_utf8_lossy(&out.stdout).split_whitespace() {
            if let Some(dir) = flag.strip_prefix("-L") {
                println!("cargo:rustc-link-search=native={dir}");
            }
        }
    }
}

/// iOS: libmpv and its dependencies are MPVKit's static xcframeworks in `ios-libs/`
/// (scripts/ios-libs.sh). Every library of the right slice is linked in.
fn link_ios() {
    use std::path::PathBuf;
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let libs = manifest.join("ios-libs");
    println!("cargo:rerun-if-changed={}", libs.display());
    let sim = std::env::var("TARGET").unwrap_or_default().ends_with("-sim");
    let slice = if sim { "ios-arm64_x86_64-simulator" } else { "ios-arm64" };
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("ios-libs");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let Ok(entries) = std::fs::read_dir(&libs) else {
        panic!("{} is missing: run scripts/ios-libs.sh first", libs.display());
    };
    for e in entries.flatten() {
        let dir = e.path().join(slice);
        let Ok(items) = std::fs::read_dir(&dir) else { continue };
        for item in items.flatten() {
            let p = item.path();
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            // Each archive goes in as lib<name>.a; libmpv2-sys asks for "mpv".
            let (archive, lib) = if let Some(fw) = name.strip_suffix(".framework") {
                (p.join(fw), if fw == "Libmpv" { "mpv".to_string() } else { fw.to_string() })
            } else if let Some(l) = name.strip_prefix("lib").and_then(|n| n.strip_suffix(".a")) {
                (p.clone(), l.to_string())
            } else {
                continue;
            };
            // Simulator slices are fat (arm64 + x86_64); rustc reads only single-architecture archives.
            let dest = out.join(format!("lib{lib}.a"));
            let thinned = std::process::Command::new("lipo")
                .arg(&archive)
                .args(["-thin", "arm64", "-output"])
                .arg(&dest)
                .status()
                .is_ok_and(|s| s.success());
            if !thinned {
                std::os::unix::fs::symlink(&archive, &dest).unwrap();
            }
            if lib != "mpv" {
                println!("cargo:rustc-link-lib=static={lib}");
            }
        }
    }
    println!("cargo:rustc-link-search=native={}", out.display());
    for fw in ["AVFoundation", "AudioToolbox", "CoreAudio", "CoreFoundation", "CoreMedia", "CoreVideo", "Metal", "VideoToolbox", "IOSurface", "QuartzCore", "UIKit", "Security", "OpenGLES"] {
        println!("cargo:rustc-link-lib=framework={fw}");
    }
    for lib in ["bz2", "c++", "expat", "iconv", "resolv", "xml2", "z"] {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
}
