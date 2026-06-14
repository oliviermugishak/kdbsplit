mod evdev;
mod ipc;
mod runtime;
mod uinput;

use anyhow::Result;
use kbdsplit_shared::DEFAULT_SOCKET_PATH;
use runtime::Daemon;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::info;
use tracing_subscriber::EnvFilter;

pub(crate) static SHUTDOWN: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_signal(_: i32) {
    SHUTDOWN.store(true, Ordering::Release);
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("kbdsplit=info".parse()?))
        .init();

    unsafe {
        libc::signal(libc::SIGTERM, handle_signal as *const () as usize);
        libc::signal(libc::SIGINT, handle_signal as *const () as usize);
    }
    info!("signal handlers installed for SIGTERM/SIGINT");

    let socket_path = std::env::args()
        .skip_while(|arg| arg != "--socket")
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET_PATH));

    let daemon = Daemon::start(socket_path)?;
    daemon.run()
}
