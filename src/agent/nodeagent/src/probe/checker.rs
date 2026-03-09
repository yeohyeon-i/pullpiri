/*
 * SPDX-FileCopyrightText: Copyright 2024 LG Electronics Inc.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Probe checker implementations for HTTP, TCP, and Exec probe types.

use tokio::time::{timeout, Duration};

/// Perform an HTTP GET probe against `http://127.0.0.1:{port}{path}`.
///
/// Returns `true` if the response status code is in the range 200–399.
/// Returns `false` on connection error, timeout, or non-2xx/3xx response.
pub async fn check_http(path: &str, port: u16, timeout_secs: u32) -> bool {
    use hyper::{Client, Uri};

    let uri_str = format!("http://127.0.0.1:{}{}", port, path);
    let uri: Uri = match uri_str.parse() {
        Ok(u) => u,
        Err(e) => {
            eprintln!("[Probe] Invalid URI {}: {}", uri_str, e);
            return false;
        }
    };

    let client = Client::new();
    let duration = Duration::from_secs(timeout_secs as u64);

    match timeout(duration, client.get(uri)).await {
        Ok(Ok(response)) => {
            let status = response.status().as_u16();
            (200..400).contains(&status)
        }
        Ok(Err(e)) => {
            eprintln!("[Probe] HTTP probe error: {}", e);
            false
        }
        Err(_) => {
            eprintln!("[Probe] HTTP probe timed out after {}s", timeout_secs);
            false
        }
    }
}

/// Perform a TCP connection probe against `127.0.0.1:{port}`.
///
/// Returns `true` if a TCP connection can be established within `timeout_secs`.
pub async fn check_tcp(port: u16, timeout_secs: u32) -> bool {
    use tokio::net::TcpStream;

    let addr = format!("127.0.0.1:{}", port);
    let duration = Duration::from_secs(timeout_secs as u64);

    match timeout(duration, TcpStream::connect(&addr)).await {
        Ok(Ok(_)) => true,
        Ok(Err(e)) => {
            eprintln!(
                "[Probe] TCP probe connection failed on port {}: {}",
                port, e
            );
            false
        }
        Err(_) => {
            eprintln!("[Probe] TCP probe timed out after {}s", timeout_secs);
            false
        }
    }
}

/// Perform an Exec probe by running `podman exec <container_id> <command>`.
///
/// Returns `true` if the command exits with code 0 within `timeout_secs`.
pub async fn check_exec(container_id: &str, command: &[String], timeout_secs: u32) -> bool {
    use tokio::process::Command;

    if command.is_empty() {
        eprintln!("[Probe] Exec probe command is empty");
        return false;
    }

    let duration = Duration::from_secs(timeout_secs as u64);

    let mut cmd = Command::new("podman");
    cmd.arg("exec").arg(container_id);
    for arg in command {
        cmd.arg(arg);
    }

    match timeout(duration, cmd.output()).await {
        Ok(Ok(output)) => {
            if output.status.success() {
                true
            } else {
                eprintln!(
                    "[Probe] Exec probe exited with non-zero code: {}",
                    output.status
                );
                false
            }
        }
        Ok(Err(e)) => {
            eprintln!("[Probe] Exec probe failed to execute: {}", e);
            false
        }
        Err(_) => {
            eprintln!("[Probe] Exec probe timed out after {}s", timeout_secs);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_check_http_invalid_port_returns_false() {
        // Port 1 is privileged and should not have an HTTP server
        let result = check_http("/", 1, 1).await;
        assert!(!result);
    }

    #[tokio::test]
    async fn test_check_tcp_closed_port_returns_false() {
        // Port 19999 is very unlikely to be open in the test environment
        let result = check_tcp(19999, 1).await;
        assert!(!result);
    }

    #[tokio::test]
    async fn test_check_exec_empty_command_returns_false() {
        let result = check_exec("some-container", &[], 5).await;
        assert!(!result);
    }

    #[tokio::test]
    async fn test_check_exec_nonexistent_container_returns_false() {
        let result = check_exec(
            "nonexistent-container-xyz",
            &["echo".to_string(), "hello".to_string()],
            5,
        )
        .await;
        assert!(!result);
    }
}
