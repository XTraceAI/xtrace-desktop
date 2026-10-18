-- The local record of product-analytics events the app tried to send: the
-- last 200 entries, newest kept. Each row holds only the event name and the
-- documented properties sent with it (see TELEMETRY.md), never content, paths
-- or identifiers of sessions. `status` is `pending` until the endpoint
-- accepted the request (`sent`) or delivery stopped for good (`failed`,
-- including work cancelled by turning telemetry off). Additive only: no
-- existing table changes.
CREATE TABLE telemetry_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recorded_at INTEGER NOT NULL CHECK(typeof(recorded_at) = 'integer' AND recorded_at >= 0),
    event TEXT NOT NULL CHECK(typeof(event) = 'text' AND length(event) BETWEEN 1 AND 64),
    properties_json TEXT NOT NULL CHECK(json_valid(properties_json) AND json_type(properties_json) = 'object'),
    status TEXT NOT NULL CHECK(status IN ('pending', 'sent', 'failed'))
);
