/*
 * SPDX-FileCopyrightText: Copyright 2024 LG Electronics Inc.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Probe module for NodeAgent self-healing.
//!
//! This module implements the liveness probe functionality that monitors
//! running containers and stops containers that fail health checks
//! above the configured failure threshold.
//!
//! ## Sub-modules
//! - `checker`: Low-level probe implementations (HTTP, TCP, Exec)
//! - `liveness`: Probe loop and high-level liveness check logic

pub mod checker;
pub mod liveness;
