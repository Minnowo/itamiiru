//! xdg-desktop-portal-itamiiru: the D-Bus FileChooser backend. Linux only.

#[cfg(target_os = "linux")]
fn main() {
    itamiiru_core::debug!("portal starting");
    eprintln!("xdg-desktop-portal-itamiiru: not implemented yet (Phase 3)");
    std::process::exit(1);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("xdg-desktop-portal-itamiiru only runs on Linux");
    std::process::exit(1);
}
