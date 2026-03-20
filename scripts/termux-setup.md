# Android Setup (Termux + Tasker)

Send voice commands to chops from your Android phone over Tailscale.

## Option 1: Termux (manual commands)

### Install

```bash
pkg install mosquitto
```

### Test connectivity

```bash
mosquitto_pub -h pop-mini -p 1884 -t voice/transcriptions \
  -m '{"text":"run echo hello from android","is_final":true}'
```

### Use the helper script

Copy `chops-send.sh` to your Termux home:

```bash
curl -o ~/chops-send.sh https://raw.githubusercontent.com/thompsonson/chops/main/scripts/chops-send.sh
chmod +x ~/chops-send.sh

# Send commands
~/chops-send.sh in chops run cargo test
~/chops-send.sh in chops tell claude fix the tests. Over.
```

## Option 2: Tasker + Termux (voice-triggered)

### Setup

1. Install [Tasker](https://play.google.com/store/apps/details?id=net.dinglisch.android.taskerm) and [Termux:Tasker](https://f-droid.org/packages/com.termux.tasker/)
2. Place `chops-send.sh` in `~/.termux/tasker/`

### Tasker Profile

**Trigger:** AutoVoice Recognized (or Tasker "Get Voice" action)

**Task:**
1. Action: Plugin > Termux:Tasker
2. Script: `chops-send.sh`
3. Arguments: `%avword` (AutoVoice recognized text)

This lets you say a command on your phone and have it execute on pop-mini.

## Option 3: Tasker MQTT Plugin (no Termux)

If you prefer not to use Termux, the [MQTT Client](https://play.google.com/store/apps/details?id=in.dc297.mqttclpro) Tasker plugin can publish directly:

1. Install MQTT Client Pro
2. Create a Tasker task:
   - Action: MQTT Publish
   - Broker: `pop-mini:1884`
   - Topic: `voice/transcriptions`
   - Payload: `{"text":"%avword","is_final":true}`

## Option 4: Web UI

Open `http://pop-mini:8080` in your phone's browser. No app install needed.

## Tailscale Note

All options require your phone to be on the same Tailscale network as pop-mini. Use the Tailscale hostname (`pop-mini`) or IP (`100.x.y.z`).
