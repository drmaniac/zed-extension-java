use serde::Serialize;
use std::net::TcpListener;
use std::process::{self, Command};
use std::time::{Duration, Instant};

#[derive(Serialize)]
pub struct TaskCommand {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
}

impl TaskCommand {
    pub fn execute(self) {
        let mut cmd = Command::new(&self.command);
        cmd.args(&self.args);
        cmd.current_dir(&self.cwd);

        // Inherit stdin/stdout/stderr
        cmd.stdin(process::Stdio::inherit());
        cmd.stdout(process::Stdio::inherit());
        cmd.stderr(process::Stdio::inherit());

        let mut child = cmd.spawn().unwrap_or_else(|e| {
            eprintln!("Failed to execute {}: {}", self.command, e);
            process::exit(1);
        });

        if crate::is_debug() {
            // Poll for the JDWP port to become available by trying to bind to it.
            // When JDWP is listening, `bind()` fails with EADDRINUSE.
            // Crucially, this does NOT establish a TCP connection to JDWP, avoiding
            // interference with the JDWP handshake later.
            let port: u16 = crate::get_debug_port().parse().unwrap_or(5005);
            let start = Instant::now();
            let timeout = Duration::from_secs(180);
            let poll_interval = Duration::from_millis(200);

            eprintln!("Waiting for JDWP port {} to become available...", port);
            loop {
                if TcpListener::bind(format!("127.0.0.1:{}", port)).is_err() {
                    eprintln!("JDWP port {} is ready", port);
                    break;
                }
                if start.elapsed() >= timeout {
                    eprintln!(
                        "Timed out after {:?} waiting for JDWP port {}",
                        timeout, port
                    );
                    break;
                }
                std::thread::sleep(poll_interval);
            }

            // Do NOT wait for the child. The debugger attaches via get_dap_binary
            // after the build task exits, so we must exit here to signal completion.
            process::exit(0);
        } else {
            let status = child.wait().unwrap_or_else(|e| {
                eprintln!("Failed to wait for {}: {}", self.command, e);
                process::exit(1);
            });
            process::exit(status.code().unwrap_or(0));
        }
    }
}
