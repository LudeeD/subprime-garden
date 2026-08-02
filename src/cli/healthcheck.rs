use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::config::Config;

/// Hits /healthz over a raw TCP connection so the distroless runtime image
/// doesn't need curl, wget, or a shell for `docker healthcheck`.
pub fn run(config: &Config) -> Result<()> {
    let addr = config.bind_addr().context("invalid server.bind address")?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .context("failed to connect to server")?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;

    let request = format!(
        "GET /healthz HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes())?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;

    if response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200") {
        Ok(())
    } else {
        let status_line = response.lines().next().unwrap_or("<empty response>");
        anyhow::bail!("healthcheck failed: {status_line}")
    }
}
