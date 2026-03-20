// Minimal service worker for PWA install support.
// No offline caching — chops needs live MQTT and API connections.
self.addEventListener('fetch', () => {});
