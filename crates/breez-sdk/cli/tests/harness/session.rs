//! Drives one CLI REPL process over piped stdin/stdout.
//!
//! Step delimiting uses a marker protocol: after each command (and its
//! scripted answers) the runner writes a bogus command like
//! `__step_end_3__`. The CLI echoes the token in its unknown-command error,
//! which marks the end of the step's output with no CLI changes.

use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin};

/// How long a transcript may stay unchanged after the marker appears before
/// the step output is considered complete (lets the marker error's trailing
/// usage lines land in the current chunk, not the next one).
const QUIESCE: Duration = Duration::from_millis(200);
const POLL: Duration = Duration::from_millis(100);

/// Append everything the reader produces to the shared transcript.
fn pump<R>(reader: R, transcript: Arc<Mutex<String>>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                    transcript.lock().expect("transcript lock").push_str(&text);
                }
            }
        }
    });
}

pub struct CliSession {
    child: Child,
    stdin: ChildStdin,
    transcript: Arc<Mutex<String>>,
    cursor: usize,
    step_counter: u32,
}

impl CliSession {
    pub fn spawn(data_dir: &Path, extra_args: &[String]) -> Result<Self> {
        let exe = env!("CARGO_BIN_EXE_cli");
        let mut child = tokio::process::Command::new(exe)
            .arg("--data-dir")
            .arg(data_dir)
            .args(extra_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("failed to spawn {exe}"))?;

        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let transcript = Arc::new(Mutex::new(String::new()));
        pump(stdout, Arc::clone(&transcript));
        pump(stderr, Arc::clone(&transcript));

        Ok(Self {
            child,
            stdin,
            transcript,
            cursor: 0,
            step_counter: 0,
        })
    }

    /// Run one command with its scripted stdin answers and return the
    /// transcript chunk it produced.
    pub async fn run_step(
        &mut self,
        cmd: &str,
        stdin_lines: &[String],
        timeout: Duration,
    ) -> Result<String> {
        self.step_counter = self.step_counter.saturating_add(1);
        let marker = format!("__step_end_{}__", self.step_counter);

        let mut input = format!("{cmd}\n");
        for line in stdin_lines {
            input.push_str(line);
            input.push('\n');
        }
        input.push_str(&marker);
        input.push('\n');
        self.stdin
            .write_all(input.as_bytes())
            .await
            .context("failed to write to CLI stdin")?;
        self.stdin.flush().await.context("failed to flush stdin")?;

        self.read_step_chunk(&marker, timeout).await
    }

    async fn read_step_chunk(&mut self, marker: &str, timeout: Duration) -> Result<String> {
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout)
            .expect("deadline overflow");

        let marker_pos = loop {
            if let Some(rel) =
                self.transcript.lock().expect("transcript lock")[self.cursor..].find(marker)
            {
                break self.cursor.saturating_add(rel);
            }
            if tokio::time::Instant::now() >= deadline {
                let tail = self.pending_transcript();
                bail!(
                    "timed out after {timeout:?} waiting for step to finish; \
                     output so far:\n{tail}"
                );
            }
            tokio::time::sleep(POLL).await;
        };

        // Wait for the marker error's trailing lines to land, then consume
        // everything up to the current end of transcript.
        let mut stable_len = self.transcript.lock().expect("transcript lock").len();
        loop {
            tokio::time::sleep(QUIESCE).await;
            let len = self.transcript.lock().expect("transcript lock").len();
            if len == stable_len {
                break;
            }
            stable_len = len;
        }

        let transcript = self.transcript.lock().expect("transcript lock");
        // The chunk ends where the marker's own output begins. The marker is
        // preceded by the CLI's error prefix on the same line, so cut at the
        // start of that line.
        let chunk_end = transcript[..marker_pos].rfind('\n').unwrap_or(0);
        let chunk = transcript[self.cursor..chunk_end].to_string();
        drop(transcript);
        self.cursor = stable_len;
        Ok(chunk)
    }

    /// Unconsumed transcript, for error messages.
    pub fn pending_transcript(&self) -> String {
        self.transcript.lock().expect("transcript lock")[self.cursor..].to_string()
    }

    pub async fn close(mut self) -> Result<()> {
        self.stdin.write_all(b"exit\n").await.ok();
        self.stdin.flush().await.ok();
        let Ok(status) = tokio::time::timeout(Duration::from_secs(30), self.child.wait()).await
        else {
            self.child.kill().await.ok();
            bail!("CLI did not exit within 30s of 'exit'");
        };
        let status = status.context("failed to wait for CLI process")?;
        if !status.success() {
            bail!("CLI exited with {status}");
        }
        Ok(())
    }
}
