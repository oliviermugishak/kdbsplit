use anyhow::{Context, Result};
use kbdsplit_shared::{ClientCommand, ServerMessage, decode_message, encode_message};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

const MAX_FRAME: usize = 8 * 1024 * 1024;

pub fn read_command(stream: &mut UnixStream) -> Result<ClientCommand> {
    let mut len = [0_u8; 4];
    stream
        .read_exact(&mut len)
        .context("failed to read IPC frame header")?;
    let len = u32::from_be_bytes(len) as usize;
    anyhow::ensure!(len <= MAX_FRAME, "IPC frame is too large");
    let mut payload = vec![0; len];
    stream
        .read_exact(&mut payload)
        .context("failed to read IPC frame payload")?;
    decode_message(&payload).context("failed to decode IPC command")
}

pub fn write_message(stream: &mut UnixStream, message: &ServerMessage) -> Result<()> {
    let frame = encode_message(message).context("failed to encode IPC response")?;
    stream
        .write_all(&frame)
        .context("failed to write IPC response")
}
