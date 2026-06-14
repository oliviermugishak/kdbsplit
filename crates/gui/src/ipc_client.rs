use anyhow::{Context, Result};
use kbdsplit_shared::{ClientCommand, ServerMessage, decode_message, encode_message};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const MAX_FRAME: usize = 8 * 1024 * 1024;

pub struct IpcClient {
    socket_path: PathBuf,
    daemon: Option<Child>,
}

impl IpcClient {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            daemon: None,
        }
    }

    pub fn request(&self, command: &ClientCommand) -> Result<ServerMessage> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .with_context(|| format!("could not connect to {}", self.socket_path.display()))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .context("failed to set IPC read timeout")?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .context("failed to set IPC write timeout")?;
        let frame = encode_message(command).context("failed to encode command")?;
        stream.write_all(&frame).context("failed to send command")?;

        let mut len = [0_u8; 4];
        stream
            .read_exact(&mut len)
            .context("failed to read response header")?;
        let len = u32::from_be_bytes(len) as usize;
        anyhow::ensure!(len <= MAX_FRAME, "IPC response is too large");
        let mut payload = vec![0; len];
        stream
            .read_exact(&mut payload)
            .context("failed to read response body")?;
        decode_message(&payload).context("failed to decode response")
    }

    pub fn ensure_daemon_started(&mut self) -> Result<()> {
        if self
            .daemon
            .as_mut()
            .is_some_and(|child| child.try_wait().ok().flatten().is_none())
        {
            return Ok(());
        }
        let Some(path) = daemon_path() else {
            anyhow::bail!("kbdsplitd was not found next to the GUI binary");
        };
        let child = Command::new(&path)
            .arg("--socket")
            .arg(&self.socket_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to start {}", path.display()))?;
        self.daemon = Some(child);
        Ok(())
    }
}

fn daemon_path() -> Option<PathBuf> {
    let current = std::env::current_exe().ok()?;
    let dir = current.parent()?;
    for name in ["kbdsplitd", "kbdsplitd.exe"] {
        let path = dir.join(name);
        if is_executable_file(&path) {
            return Some(path);
        }
    }
    None
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}
