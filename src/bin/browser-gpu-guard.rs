//! Polling mitigation for browser DMA-BUF leaks and host GPU memory pressure.
#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
#[path = "browser-gpu-guard/linux.rs"]
mod linux;

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    linux::run()
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("browser-gpu-guard requires Linux");
    std::process::ExitCode::FAILURE
}
