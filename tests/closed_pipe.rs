//! A command whose stdout reader goes away must end quietly with code 0
//! instead of printing a `failed printing to stdout: Broken pipe` panic.

#![cfg(unix)]

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_texforge");

#[test]
fn printing_into_a_closed_pipe_exits_quietly() {
    // A socket with no peer fails every write with EPIPE at once. Draining a
    // byte from a piped stdout first would let `skill` (under a pipe buffer
    // of output) land entirely in the buffer and exit 0 even without the
    // closed-pipe handling, so the read end is gone before the child writes.
    let (reader, writer) = UnixStream::pair().expect("socket pair for stdout");
    drop(reader);
    let writer: OwnedFd = writer.into();

    let output = Command::new(BIN)
        .arg("skill")
        .stdout(Stdio::from(writer))
        .stderr(Stdio::piped())
        .output()
        .expect("run the texforge binary");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "a closed output pipe must exit 0, got {:?} with stderr:\n{stderr}",
        output.status
    );
    assert!(
        output.stderr.is_empty(),
        "a closed output pipe must print nothing to stderr, got:\n{stderr}"
    );
}
