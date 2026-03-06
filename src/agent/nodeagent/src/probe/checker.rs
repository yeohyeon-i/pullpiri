/*
 * SPDX-FileCopyrightText: Copyright 2024 LG Electronics Inc.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Low-level probe checker implementations.
//!
//! This module provides the actual probe execution logic for each supported
//! probe type:
//! - **HTTP**: sends an HTTP GET request and checks for a 2xx/3xx response
//! - **TCP**: attempts to open a TCP connection to the given host:port
//! - **Exec**: runs a command inside the container via `podman exec` and
//!   checks for exit code 0

use std::time::Duration;
use tokio::net::TcpStream;

/// Performs an HTTP GET probe against `http://host:port/path`.
///
/// Returns `true` if the response status code is in the range `[200, 400)`.
/// Any network error, timeout, or non-2xx/3xx status is treated as failure.
pub async fn check_http(host: &str, port: u16, path: &str, timeout_secs: u32) -> bool {
    use hyper::{Client, Uri};

    let uri_str = format!("http://{}:{}{}", host, port, path);
    let uri = match uri_str.parse::<Uri>() {
        Ok(u) => u,
        Err(_) => return false,
    };

    let client = Client::new();
    let result =
        tokio::time::timeout(Duration::from_secs(timeout_secs as u64), client.get(uri)).await;

    match result {
        Ok(Ok(response)) => {
            let status = response.status().as_u16();
            (200..400).contains(&status)
        }
        _ => false,
    }
}

/// Performs a TCP connection probe against `host:port`.
///
/// Returns `true` if a TCP connection can be established within `timeout_secs`.
pub async fn check_tcp(host: &str, port: u16, timeout_secs: u32) -> bool {
    let addr = format!("{}:{}", host, port);
    let result = tokio::time::timeout(
        Duration::from_secs(timeout_secs as u64),
        TcpStream::connect(&addr),
    )
    .await;
    matches!(result, Ok(Ok(_)))
}

/// Performs an Exec probe by running `command` inside `container_id` via
/// `podman exec`.
///
/// Returns `true` if the command exits with code 0. An empty command,
/// a timeout, or any other error is treated as failure.
pub async fn check_exec(container_id: &str, command: &[String], timeout_secs: u32) -> bool {
    if command.is_empty() {
        return false;
    }

    let result = tokio::time::timeout(
        Duration::from_secs(timeout_secs as u64),
        tokio::process::Command::new("podman")
            .arg("exec")
            .arg(container_id)
            .args(command)
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => output.status.success(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// HTTP probe against a port with no listener should return false.
    #[tokio::test]
    async fn test_check_http_no_server() {
        let result = check_http("127.0.0.1", 59997, "/health", 1).await;
        assert!(!result);
    }

    /// TCP probe against a port with no listener should return false.
    #[tokio::test]
    async fn test_check_tcp_no_listener() {
        let result = check_tcp("127.0.0.1", 59998, 1).await;
        assert!(!result);
    }

    /// An empty command slice should immediately return false without invoking podman.
    #[tokio::test]
    async fn test_check_exec_empty_command() {
        let result = check_exec("some-container", &[], 1).await;
        assert!(!result);
    }

    /// Exec probe against a nonexistent container should return false.
    /// If `podman` is not installed the process invocation itself will fail,
    /// which is also mapped to false.
    #[tokio::test]
    async fn test_check_exec_nonexistent_container() {
        let result = check_exec("nonexistent-container-id-xyz", &["true".to_string()], 2).await;
        assert!(!result);
    }
}
