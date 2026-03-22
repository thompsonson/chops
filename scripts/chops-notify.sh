#!/bin/bash
# Publish a toast notification to chops via MQTT.
# Usage: chops-notify.sh <source> <level> <message>
#   e.g. chops-notify.sh claude info "Waiting for permission"
#
# Levels: info, ok, warn, error
# Sources: claude, pi, agent, plugin, etc.
#
# Environment variables:
#   CHOPS_HOST      - MQTT broker hostname (default: pop-mini)
#   CHOPS_MQTT_PORT - MQTT broker port (default: 1884)

set -euo pipefail

HOST="${CHOPS_HOST:-pop-mini}"
PORT="${CHOPS_MQTT_PORT:-1884}"
SOURCE="${1:?usage: chops-notify.sh <source> <level> <message>}"
LEVEL="${2:?usage: chops-notify.sh <source> <level> <message>}"
MESSAGE="${3:?usage: chops-notify.sh <source> <level> <message>}"

mosquitto_pub -h "$HOST" -p "$PORT" -t agent/responses \
  -m "{\"source\":\"$SOURCE\",\"type\":\"toast\",\"level\":\"$LEVEL\",\"message\":\"$MESSAGE\"}"
