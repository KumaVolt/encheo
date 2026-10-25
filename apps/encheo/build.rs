fn main() {
    slint_build::compile("ui/app.slint").expect("compile ui");

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
