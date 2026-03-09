/*
 * SPDX-FileCopyrightText: Copyright 2024 LG Electronics Inc.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Liveness probe loop for NodeAgent self-healing.
//!
//! This module implements the liveness probe loop that:
//! 1. Runs every second and queries Podman for running containers
//! 2. Looks up each container's desired state (including probe configuration)
//!    in the shared in-memory cache
//! 3. Respects `initial_delay_seconds` (waits after container start) and
//!    `period_seconds` (interval between probes)
//! 4. Executes the configured probe type (HTTP / TCP / Exec)
//! 5. Increments a per-container failure counter on each failure and resets
//!    it on success
//! 6. Calls `podman stop` when the failure counter reaches `failure_threshold`

use crate::desired_state::{DesiredState, LivenessProbe, ProbeType};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::Mutex;

/// Per-container probe tracking state (held entirely in memory).
///
/// `ProbeState` is ephemeral: it is not persisted to disk or etcd. If the
/// NodeAgent restarts, all probe states are lost and containers restart their
/// failure counts from zero on the next probe-loop iteration.
struct ProbeState {
    /// Number of consecutive probe failures since the last success.
    failure_count: u8,
    /// Time the container was started (sourced from Podman inspect).
    container_started_at: SystemTime,
    /// Time the last probe was executed, used for `period_seconds` gating.
    last_probe_at: Option<SystemTime>,
}

/// Runs the liveness probe loop indefinitely.
///
/// Every second this function:
/// 1. Fetches the list of running containers from Podman
/// 2. For each container with a liveness probe configured in
///    `desired_states_cache`, checks whether it is time to run the probe
/// 3. Executes the probe and updates the failure counter
/// 4. Stops the container if the failure counter reaches `failure_threshold`
///
/// The `desired_states_cache` is shared with the gRPC receiver so probe
/// configurations are always up to date.
pub async fn probe_loop(desired_states_cache: Arc<Mutex<HashMap<String, DesiredState>>>) {
    use crate::resource::container::{get_inspect, get_list};
    use tokio::time::sleep;

    let mut probe_states: HashMap<String, ProbeState> = HashMap::new();

    loop {
        // Snapshot the desired states and release the lock immediately.
        let desired_states = {
            let cache = desired_states_cache.lock().await;
            cache.clone()
        };

        // Fetch running containers from Podman.
        let running_containers = match get_list().await {
            Ok(containers) => containers
                .into_iter()
                .filter(|c| c.State == "running")
                .map(|c| c.Id)
                .collect::<Vec<_>>(),
            Err(e) => {
                eprintln!("[Probe] Failed to list containers: {:?}", e);
                sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        // Remove probe state for containers that are no longer running.
        probe_states.retain(|id, _| running_containers.contains(id));

        let now = SystemTime::now();

        for container_id in &running_containers {
            // Find the matching desired state by container ID.
            let desired = match desired_states
                .values()
                .find(|d| &d.container_id == container_id)
            {
                Some(d) => d,
                None => continue,
            };

            // Skip if no liveness probe is configured.
            let liveness = match desired
                .probe_config
                .as_ref()
                .and_then(|pc| pc.liveness.as_ref())
            {
                Some(l) => l,
                None => continue,
            };

            // Initialize probe state for newly discovered containers.
            if !probe_states.contains_key(container_id) {
                let started_at = match get_inspect(container_id).await {
                    Ok(inspect) => parse_rfc3339(&inspect.State.StartedAt).unwrap_or_else(|| {
                        eprintln!(
                            "[Probe] Could not parse StartedAt '{}' for container '{}'; \
                             using current time as fallback (initial_delay_seconds may be inaccurate)",
                            inspect.State.StartedAt, container_id
                        );
                        SystemTime::now()
                    }),
                    Err(e) => {
                        eprintln!(
                            "[Probe] Failed to inspect container '{}': {:?}; \
                             using current time as container_started_at",
                            container_id, e
                        );
                        SystemTime::now()
                    }
                };
                probe_states.insert(
                    container_id.clone(),
                    ProbeState {
                        failure_count: 0,
                        container_started_at: started_at,
                        last_probe_at: None,
                    },
                );
            }

            let probe_state = match probe_states.get_mut(container_id) {
                Some(ps) => ps,
                None => continue,
            };

            // Respect initial_delay_seconds.
            let elapsed_since_start = now
                .duration_since(probe_state.container_started_at)
                .unwrap_or(Duration::ZERO);
            if elapsed_since_start < Duration::from_secs(liveness.initial_delay_seconds as u64) {
                continue;
            }

            // Respect period_seconds.
            if let Some(last_probe) = probe_state.last_probe_at {
                let elapsed_since_probe = now.duration_since(last_probe).unwrap_or(Duration::ZERO);
                if elapsed_since_probe < Duration::from_secs(liveness.period_seconds as u64) {
                    continue;
                }
            }

            // Build probe description and pod name for logging.
            let pod_name = desired.pod_name.clone();
            let probe_desc = format_probe_desc(&liveness.probe_type);

            println!(
                "[Probe] Checking liveness probe for container '{}'",
                pod_name
            );

            // Execute the liveness probe.
            let success = check_liveness_probe(container_id, liveness).await;
            probe_state.last_probe_at = Some(now);

            if success {
                println!(
                    "[Probe] Liveness probe for container '{}': {} - Success",
                    pod_name, probe_desc
                );
                probe_state.failure_count = 0;
            } else {
                probe_state.failure_count = probe_state.failure_count.saturating_add(1);
                eprintln!(
                    "[Probe] Liveness probe failed ({}/{})",
                    probe_state.failure_count, liveness.failure_threshold
                );
                if probe_state.failure_count >= liveness.failure_threshold {
                    println!(
                        "[NodeAgent] Stopping container '{}' due to liveness probe failure",
                        pod_name
                    );
                    stop_container(container_id).await;
                    probe_states.remove(container_id);
                }
            }
        }

        sleep(Duration::from_secs(1)).await;
    }
}

/// Executes the appropriate probe type for the given container and liveness
/// configuration.
///
/// Returns `true` if the probe succeeds, `false` on any failure or timeout.
pub async fn check_liveness_probe(container_id: &str, liveness: &LivenessProbe) -> bool {
    use super::checker;

    match &liveness.probe_type {
        ProbeType::Http { path, port } => {
            checker::check_http("localhost", *port, path, liveness.timeout_seconds).await
        }
        ProbeType::Tcp { port } => {
            checker::check_tcp("127.0.0.1", *port, liveness.timeout_seconds).await
        }
        ProbeType::Exec { command } => {
            checker::check_exec(container_id, command, liveness.timeout_seconds).await
        }
    }
}

/// Returns a human-readable description of the probe type for log messages.
fn format_probe_desc(probe_type: &ProbeType) -> String {
    match probe_type {
        ProbeType::Http { path, port } => format!("HTTP GET {} on port {}", path, port),
        ProbeType::Tcp { port } => format!("TCP port {}", port),
        ProbeType::Exec { command } => format!("Exec {:?}", command),
    }
}

/// Stops a container by its Podman container ID using the Podman REST API.
async fn stop_container(container_id: &str) {
    use hyper::Body;

    let path = format!("/v4.0.0/libpod/containers/{}/stop", container_id);
    match crate::runtime::podman::post(&path, Body::empty()).await {
        Ok(_) => println!("[Probe] Container '{}' stopped successfully", container_id),
        Err(e) => eprintln!(
            "[Probe] Failed to stop container '{}': {:?}",
            container_id, e
        ),
    }
}

/// Parses an RFC 3339 / ISO 8601 timestamp string such as
/// `"2024-01-15T10:30:45.123456789Z"` into a [`SystemTime`].
///
/// # Timezone handling
/// This parser extracts the `HH:MM:SS` portion of the time field directly
/// (the first 8 bytes after the `T` separator) and ignores any timezone
/// offset (`Z`, `+HH:MM`, or `-HH:MM`).  Timestamps are treated as UTC.
/// Podman typically returns UTC timestamps with a `Z` suffix, so this
/// assumption is correct in practice.
///
/// Returns `None` if the string cannot be parsed.
fn parse_rfc3339(s: &str) -> Option<SystemTime> {
    let t_pos = s.find('T')?;
    let date_str = &s[..t_pos];
    let rest = &s[t_pos + 1..];

    // Parse YYYY-MM-DD
    let mut date_iter = date_str.splitn(3, '-');
    let year: i64 = date_iter.next()?.parse().ok()?;
    let month: i64 = date_iter.next()?.parse().ok()?;
    let day: i64 = date_iter.next()?.parse().ok()?;

    // The time portion always begins with "HH:MM:SS" (8 bytes).
    // Slicing the first 8 bytes handles all suffix variants:
    //   "10:30:45Z", "10:30:45.123Z", "10:30:45+00:00", "10:30:45-05:00",
    //   "10:30:45.123456789+00:00", etc.
    if rest.len() < 8 {
        return None;
    }
    let time_str = &rest[..8];

    // Parse HH:MM:SS
    let mut time_iter = time_str.splitn(3, ':');
    let hour: i64 = time_iter.next()?.parse().ok()?;
    let minute: i64 = time_iter.next()?.parse().ok()?;
    let second: i64 = time_iter.next()?.parse().ok()?;

    let days = days_since_unix_epoch(year, month, day)?;
    let total_secs = days * 86400 + hour * 3600 + minute * 60 + second;

    if total_secs < 0 {
        return None;
    }

    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(total_secs as u64))
}

/// Returns the number of days between the Unix epoch (`1970-01-01`) and the
/// given Gregorian calendar date.
///
/// Returns `None` if the arguments are out of range or if `year < 1970`.
fn days_since_unix_epoch(year: i64, month: i64, day: i64) -> Option<i64> {
    if year < 1970 || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    // Days in each month for a common (non-leap) year.
    let days_in_month: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

    let mut total_days: i64 = 0;

    // Accumulate complete years since 1970.
    for y in 1970..year {
        total_days += if is_leap_year(y) { 366 } else { 365 };
    }

    // Accumulate complete months in the current year.
    for m in 1..month {
        let m_idx = (m - 1) as usize;
        total_days += days_in_month[m_idx];
        // February gets an extra day in leap years.
        if m == 2 && is_leap_year(year) {
            total_days += 1;
        }
    }

    // Add remaining days within the current month (0-indexed).
    total_days += day - 1;

    Some(total_days)
}

/// Returns `true` if `year` is a leap year in the proleptic Gregorian
/// calendar.
fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desired_state::{DesiredState, LivenessProbe, ProbeConfig, ProbeType};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tokio::time::{timeout, Duration};

    // ── Helper ──────────────────────────────────────────────────────────────

    fn make_liveness(probe_type: ProbeType) -> LivenessProbe {
        LivenessProbe {
            probe_type,
            initial_delay_seconds: 0,
            period_seconds: 1,
            timeout_seconds: 1,
            failure_threshold: 3,
        }
    }

    // ── format_probe_desc ────────────────────────────────────────────────────

    #[test]
    fn test_format_probe_desc_http() {
        let desc = format_probe_desc(&ProbeType::Http {
            path: "/health".to_string(),
            port: 8080,
        });
        assert_eq!(desc, "HTTP GET /health on port 8080");
    }

    #[test]
    fn test_format_probe_desc_tcp() {
        let desc = format_probe_desc(&ProbeType::Tcp { port: 6379 });
        assert_eq!(desc, "TCP port 6379");
    }

    #[test]
    fn test_format_probe_desc_exec() {
        let desc = format_probe_desc(&ProbeType::Exec {
            command: vec!["cat".to_string(), "/tmp/healthy".to_string()],
        });
        assert!(desc.contains("cat") && desc.contains("/tmp/healthy"));
    }

    // ── parse_rfc3339 ────────────────────────────────────────────────────────

    #[test]
    fn test_parse_rfc3339_unix_epoch() {
        let result = parse_rfc3339("1970-01-01T00:00:00Z");
        assert_eq!(result, Some(SystemTime::UNIX_EPOCH));
    }

    #[test]
    fn test_parse_rfc3339_with_nanoseconds() {
        // Nanoseconds should be stripped; the result must be parseable.
        let result = parse_rfc3339("2024-01-15T10:30:45.123456789Z");
        assert!(result.is_some());
    }

    #[test]
    fn test_parse_rfc3339_with_positive_offset() {
        // Positive UTC offset should be parsed the same as UTC (offset is ignored).
        let result = parse_rfc3339("2024-06-01T08:00:00.000000000+00:00");
        assert!(result.is_some());
    }

    #[test]
    fn test_parse_rfc3339_with_negative_offset() {
        // Negative UTC offset must not break parsing (offset is ignored).
        let result = parse_rfc3339("2024-06-01T08:00:00-05:00");
        assert!(result.is_some());
    }

    #[test]
    fn test_parse_rfc3339_negative_offset_no_fractions() {
        let result = parse_rfc3339("1970-01-01T00:00:00-00:00");
        assert_eq!(result, Some(SystemTime::UNIX_EPOCH));
    }

    #[test]
    fn test_parse_rfc3339_invalid() {
        assert!(parse_rfc3339("not-a-date").is_none());
        assert!(parse_rfc3339("").is_none());
    }

    // ── days_since_unix_epoch ────────────────────────────────────────────────

    #[test]
    fn test_days_unix_epoch() {
        assert_eq!(days_since_unix_epoch(1970, 1, 1), Some(0));
    }

    #[test]
    fn test_days_next_day() {
        assert_eq!(days_since_unix_epoch(1970, 1, 2), Some(1));
    }

    #[test]
    fn test_days_first_day_of_february() {
        // January has 31 days.
        assert_eq!(days_since_unix_epoch(1970, 2, 1), Some(31));
    }

    #[test]
    fn test_days_first_day_of_1971() {
        // 1970 is not a leap year → 365 days.
        assert_eq!(days_since_unix_epoch(1971, 1, 1), Some(365));
    }

    #[test]
    fn test_days_2024_jan_01() {
        // Verify against a known Unix timestamp: 2024-01-01 00:00:00 UTC = 1704067200
        // 1704067200 / 86400 = 19723
        assert_eq!(days_since_unix_epoch(2024, 1, 1), Some(19723));
    }

    #[test]
    fn test_days_invalid_year() {
        assert!(days_since_unix_epoch(1969, 1, 1).is_none());
    }

    #[test]
    fn test_days_invalid_month() {
        assert!(days_since_unix_epoch(2024, 0, 1).is_none());
        assert!(days_since_unix_epoch(2024, 13, 1).is_none());
    }

    // ── is_leap_year ─────────────────────────────────────────────────────────

    #[test]
    fn test_is_leap_year() {
        assert!(is_leap_year(2000));
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(1900));
        assert!(!is_leap_year(2023));
    }

    // ── check_liveness_probe ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_check_liveness_probe_http_no_server() {
        let probe = make_liveness(ProbeType::Http {
            path: "/health".to_string(),
            port: 59990,
        });
        assert!(!check_liveness_probe("test-container", &probe).await);
    }

    #[tokio::test]
    async fn test_check_liveness_probe_tcp_no_listener() {
        let probe = make_liveness(ProbeType::Tcp { port: 59991 });
        assert!(!check_liveness_probe("test-container", &probe).await);
    }

    #[tokio::test]
    async fn test_check_liveness_probe_exec_nonexistent_container() {
        let probe = make_liveness(ProbeType::Exec {
            command: vec!["true".to_string()],
        });
        // nonexistent container → podman exec fails → false
        assert!(!check_liveness_probe("nonexistent-xyz-container", &probe).await);
    }

    // ── probe_loop ───────────────────────────────────────────────────────────

    /// The probe loop must not panic when the cache is empty.
    #[tokio::test]
    async fn test_probe_loop_empty_cache_no_panic() {
        let cache: Arc<Mutex<HashMap<String, DesiredState>>> = Arc::new(Mutex::new(HashMap::new()));
        // The loop runs forever; a short timeout is sufficient to verify
        // there is no immediate panic.
        let result = timeout(Duration::from_millis(300), probe_loop(cache)).await;
        assert!(result.is_err(), "probe_loop should run indefinitely");
    }

    /// The probe loop must not panic when a desired state exists but has no
    /// probe configuration (the container should simply be skipped).
    #[tokio::test]
    async fn test_probe_loop_no_probe_config_skips_gracefully() {
        let cache: Arc<Mutex<HashMap<String, DesiredState>>> = Arc::new(Mutex::new(HashMap::new()));
        {
            let mut c = cache.lock().await;
            let mut state = DesiredState::new("test-pod".to_string());
            state.container_id = "test-container-id".to_string();
            state.probe_config = None;
            c.insert("test-pod".to_string(), state);
        }
        let result = timeout(Duration::from_millis(300), probe_loop(cache)).await;
        assert!(result.is_err());
    }

    /// The probe loop must handle a desired state that has a liveness probe
    /// configured but whose container is not found in the running list.
    #[tokio::test]
    async fn test_probe_loop_with_probe_config_no_running_container() {
        let cache: Arc<Mutex<HashMap<String, DesiredState>>> = Arc::new(Mutex::new(HashMap::new()));
        {
            let mut c = cache.lock().await;
            let mut state = DesiredState::new("probe-pod".to_string());
            state.container_id = "not-a-real-container-id".to_string();
            state.probe_config = Some(ProbeConfig {
                liveness: Some(LivenessProbe {
                    probe_type: ProbeType::Http {
                        path: "/health".to_string(),
                        port: 8080,
                    },
                    initial_delay_seconds: 0,
                    period_seconds: 1,
                    timeout_seconds: 1,
                    failure_threshold: 3,
                }),
            });
            c.insert("probe-pod".to_string(), state);
        }
        let result = timeout(Duration::from_millis(300), probe_loop(cache)).await;
        assert!(result.is_err());
    }
}
