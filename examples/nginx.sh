#!/bin/bash
# SPDX-FileCopyrightText: Copyright 2024 LG Electronics Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Deploy nginx with liveness probe enabled.
# The NodeAgent will run an HTTP GET / on port 80 every 10 seconds and stop
# the container after 3 consecutive failures.
#
# Usage: cd examples && ./nginx.sh
#
# Verify probes are running:
#   journalctl -u nodeagent -f | grep -i probe

BODY=$(< ./resources/nginx-probe.yaml)

curl -X POST 'http://127.0.0.1:47099/api/artifact' \
--header 'Content-Type: text/plain' \
--data "${BODY}"
