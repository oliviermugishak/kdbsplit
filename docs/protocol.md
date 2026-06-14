# IPC Protocol

Transport: Unix domain socket at `/tmp/kbdsplit.sock`.

Frames are length-prefixed JSON:

```text
u32 big-endian payload length
serde_json payload
```

Commands are defined in `crates/shared/src/protocol.rs`:

- `GetSnapshot`
- `AssignDevice`
- `UnassignSlot`
- `LockDevice`
- `UnlockDevice`
- `SaveProfile`
- `LoadProfile`
- `SetBinding`
- `InjectTestAction`
- `Shutdown`

The normal response is either `Snapshot`, `Ack`, or `Error`.
