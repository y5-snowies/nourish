//! Where a launched application's output goes: the journal, not our log.
//!
//! Inheriting the compositor's stdio is descriptor inheritance, fixed at `exec` and
//! unaffected by anything done afterwards — adopting the process into a systemd scope
//! moves its cgroup and nothing else. So an app launched that way writes into the
//! compositor's log for its whole life, which is how a browser's chatter ends up
//! interleaved with compositor records that are supposed to be about the compositor.
//!
//! A systemd SERVICE does not have that problem, because systemd connects the child to
//! journald before `exec`. This is that step, done in-process: a stream socket to
//! journald, opened per launch, which every write is logged from.
//!
//! It is the protocol `sd_journal_stream_fd` speaks, implemented directly rather than
//! by linking libsystemd for one function. Seven newline-terminated header fields, then
//! the socket is an ordinary stream.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::process::Stdio;

/// journald's stdout/stderr stream socket. Absent exactly when journald is.
const SOCKET: &str = "/run/systemd/journal/stdout";

/// syslog severities, for the two streams an application gets.
const INFO: u8 = 6;
const WARNING: u8 = 4;

/// Stdio for a launched application: `(stdin, stdout, stderr)`.
///
/// `stdin` is always null, which is what a service gets. An app launched from a
/// desktop has no terminal to read and inheriting ours lets it compete for the
/// compositor's input.
///
/// The two output streams go to the journal under `identifier`, tagged `info` and
/// `warning` so an app's stderr is visible without being an error — journald has no
/// notion of "this stream is untrusted", and marking every app's stderr as an error
/// would make the log useless.
///
/// FALLS BACK to null, never to inheriting. Inheriting is the behaviour this exists to
/// remove, and a fallback that reinstates it means an application's output still lands in
/// the compositor's log on exactly the machines nobody is watching closely. Losing the
/// output is the lesser harm: it is the application's own, the compositor's records stay
/// about the compositor, and anything that needs to see it can be run from a terminal.
///
/// All or nothing, deliberately. A half-connected pair would put output in the journal
/// and errors somewhere else, which is worse to debug than either one alone.
pub fn stdio(identifier: &str) -> (Stdio, Stdio, Stdio) {
    match (stream(identifier, INFO), stream(identifier, WARNING)) {
        (Some(out), Some(err)) => (Stdio::null(), out, err),
        _ => {
            info!("journald unavailable; discarding {identifier}'s output");
            (Stdio::null(), Stdio::null(), Stdio::null())
        }
    }
}

/// One journald stream, or `None` if the socket is not there or refuses the header.
///
/// The header is positional and every field must be present: the identifier to log
/// under, the unit to attribute to (empty — the scope the process is adopted into is
/// what actually names it), the default severity, and then four flags. `level_prefix`
/// is off, so a line beginning with something like `<3>` is text rather than a severity
/// the application gets to choose. The three forwarding flags are off because that is
/// journald's own business, configured centrally, not a per-app decision.
fn stream(identifier: &str, priority: u8) -> Option<Stdio> {
    let mut socket = UnixStream::connect(SOCKET).ok()?;
    let header = format!("{identifier}\n\n{priority}\n0\n0\n0\n0\n");
    socket.write_all(header.as_bytes()).ok()?;
    Some(Stdio::from(std::os::fd::OwnedFd::from(socket)))
}
