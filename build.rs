#[cfg(target_os = "windows")]
fn main() {
    let icon_path = std::path::Path::new("assets").join("app-icon.ico");
    println!("cargo:rerun-if-changed={}", icon_path.display());

    let mut res = winres::WindowsResource::new();

    if icon_path.exists() {
        res.set_icon(icon_path.to_string_lossy().as_ref());
    } else {
        println!(
            "cargo:warning=Windows app icon not found at {}",
            icon_path.display()
        );
    }

    res.set("ProductName", "WakeMATE Companion");
    res.set(
        "FileDescription",
        "WakeMATE Companion - PC wake-up and remote control companion",
    );
    res.set("CompanyName", "Marco Macias");
    res.set("LegalCopyright", "Copyright (c) 2026 Marco Macias");
    res.set("OriginalFilename", "wakemate-companion.exe");
    res.set("InternalName", "wakemate-companion");

    let version = pack_version(env!("CARGO_PKG_VERSION"));
    res.set_version_info(winres::VersionInfo::FILEVERSION, version);
    res.set_version_info(winres::VersionInfo::PRODUCTVERSION, version);

    if let Err(error) = res.compile() {
        panic!("failed to compile Windows resources: {error}");
    }
}

/// Packs a "major.minor.patch" Cargo version into the u64 layout Windows
/// version resources expect: four u16 fields (major, minor, patch, build)
/// packed high-to-low into one u64.
#[cfg(target_os = "windows")]
fn pack_version(cargo_version: &str) -> u64 {
    let mut parts = cargo_version
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0));
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    let patch = parts.next().unwrap_or(0);
    (major << 48) | (minor << 32) | (patch << 16)
}

#[cfg(not(target_os = "windows"))]
fn main() {}
