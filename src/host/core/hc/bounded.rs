//! A child's output, read within a deadline and a size limit
//! (herdr-panorama `host::command::read`). The child is killed past either.

use std::io::{ErrorKind, Read};
use std::process::{Command, Output, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};

/// `command`'s output, or why there is none.
pub fn read(mut command: Command, timeout: Duration, limit: usize) -> Result<Output, &'static str> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "hc unavailable")?;
    let mut stdout = child.stdout.take().ok_or("hc output unavailable")?;
    let mut stderr = child.stderr.take().ok_or("hc output unavailable")?;
    let setup = fcntl_getfl(&stdout)
        .and_then(|f| fcntl_setfl(&stdout, f | OFlags::NONBLOCK))
        .and_then(|()| fcntl_getfl(&stderr))
        .and_then(|f| fcntl_setfl(&stderr, f | OFlags::NONBLOCK));
    if setup.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err("hc output setup failed");
    }
    let deadline = Instant::now() + timeout;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let (mut out_eof, mut err_eof) = (false, false);
    let result = 'poll: loop {
        let mut chunk = [0_u8; 8192];
        for (reader, bytes, eof) in [
            (&mut stdout as &mut dyn Read, &mut out, &mut out_eof),
            (&mut stderr as &mut dyn Read, &mut err, &mut err_eof),
        ] {
            match reader.read(&mut chunk) {
                Ok(0) => *eof = true,
                Ok(n) => bytes.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(_) => break 'poll Err("hc output unreadable"),
            }
        }
        if out.len() + err.len() > limit {
            break Err("hc output exceeds limit");
        }
        match child.try_wait() {
            Ok(Some(status)) if out_eof && err_eof => break Ok(status),
            Err(_) => break Err("hc wait failed"),
            _ if Instant::now() >= deadline => break Err("hc read timed out"),
            _ => sleep(Duration::from_millis(1)),
        }
    };
    if result.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    Ok(Output {
        status: result?,
        stdout: out,
        stderr: err,
    })
}
