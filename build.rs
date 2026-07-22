#[cfg(target_os = "windows")]
fn main() {
    let icon_path = std::path::Path::new("assets").join("app-icon.ico");
    println!("cargo:rerun-if-changed={}", icon_path.display());

    if !icon_path.exists() {
        println!(
            "cargo:warning=Windows app icon not found at {}",
            icon_path.display()
        );
        return;
    }

    let mut res = winres::WindowsResource::new();
    res.set_icon(icon_path.to_string_lossy().as_ref());

    if let Err(error) = res.compile() {
        panic!("failed to compile Windows resources: {error}");
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {}
