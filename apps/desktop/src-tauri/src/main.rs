// Prevents an additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Elevated TUN helper of the VPN apps mode (this exe re-run via UAC).
    if let Some(code) = lc_vpn::run_tun_helper_from_args() {
        std::process::exit(code);
    }
    desktop_lib::run()
}
