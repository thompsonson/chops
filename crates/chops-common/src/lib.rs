pub const DEFAULT_MQTT_HOST: &str = "localhost";
pub const DEFAULT_MQTT_PORT: u16 = 1884;
pub const MQTT_KEEP_ALIVE_SECS: u64 = 30;
pub const MQTT_RECONNECT_DELAY_SECS: u64 = 2;
pub const MQTT_QUEUE_CAPACITY: usize = 10;

pub fn mqtt_port() -> u16 {
    std::env::var("CHOPS_MQTT_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MQTT_PORT)
}
