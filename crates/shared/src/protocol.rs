use crate::types::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_SOCKET_PATH: &str = "/tmp/kbdsplit.sock";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientCommand {
    GetSnapshot,
    AssignDevice {
        device_id: DeviceId,
        slot: ControllerSlot,
    },
    UnassignSlot {
        slot: ControllerSlot,
    },
    LockDevice {
        device_id: DeviceId,
    },
    UnlockDevice {
        device_id: DeviceId,
    },
    SaveProfile,
    LoadProfile {
        name: String,
    },
    CreateProfile {
        name: String,
    },
    DeleteProfile {
        name: String,
    },
    SetBinding {
        slot: ControllerSlot,
        binding: KeyBinding,
    },
    /// Start capturing the next key press for a specific controller action
    StartBindingCapture {
        slot: ControllerSlot,
        action: ControllerAction,
    },
    /// Cancel any pending binding capture
    CancelBindingCapture,
    InjectTestAction {
        slot: ControllerSlot,
        action: ControllerAction,
        pressed: bool,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    Snapshot(AppSnapshot),
    Ack,
    Error(String),
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("message is larger than the maximum IPC frame")]
    FrameTooLarge,
    #[error("failed to encode message: {0}")]
    Encode(#[from] serde_json::Error),
}

pub fn encode_message<T: Serialize>(message: &T) -> Result<Vec<u8>, ProtocolError> {
    let payload = serde_json::to_vec(message)?;
    if payload.len() > u32::MAX as usize {
        return Err(ProtocolError::FrameTooLarge);
    }
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub fn decode_message<T: for<'de> Deserialize<'de>>(
    payload: &[u8],
) -> Result<T, serde_json::Error> {
    serde_json::from_slice(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let cmd = ClientCommand::GetSnapshot;
        let encoded = encode_message(&cmd).unwrap();
        let decoded: ClientCommand = decode_message(&encoded[4..]).unwrap();
        assert_eq!(decoded, cmd);
    }

    #[test]
    fn test_encode_decode_assign() {
        let cmd = ClientCommand::AssignDevice {
            device_id: DeviceId("test-device".to_owned()),
            slot: ControllerSlot::Two,
        };
        let encoded = encode_message(&cmd).unwrap();
        let decoded: ClientCommand = decode_message(&encoded[4..]).unwrap();
        assert_eq!(decoded, cmd);
    }
}
