//! Typed, host-owned interactive terminal lifecycle with bounded byte-stream buffering.

mod manager;

pub use manager::{
    MAX_INPUT_BYTES, MAX_OPEN_TERMINALS_PER_HOST, OUTPUT_BATCH_BYTES, OUTPUT_CHUNK_BYTES,
    OUTPUT_QUEUE_CAPACITY, TerminalChannel, TerminalConnector, TerminalManager, TerminalRead,
};
