#!/bin/bash
# Send a command to chops via MQTT.
# Usage: chops-send.sh in chops run cargo test
#
# Environment variables:
#   CHOPS_HOST      - MQTT broker hostname (default: pop-mini)
#   CHOPS_MQTT_PORT - MQTT broker port (default: 1884)

set -euo pipefail

HOST="${CHOPS_HOST:-pop-mini}"
PORT="${CHOPS_MQTT_PORT:-1884}"
TEXT="$*"

if [ -z "$TEXT" ]; then
  echo "Usage: chops-send.sh <command>"
  echo "  e.g. chops-send.sh in chops run cargo test"
  exit 1
fi

mosquitto_pub -h "$HOST" -p "$PORT" -t voice/transcriptions \
  -m "{\"text\":\"$TEXT\",\"is_final\":true}"
