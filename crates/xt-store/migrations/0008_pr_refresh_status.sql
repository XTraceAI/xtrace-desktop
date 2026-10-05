-- Pull-request refresh status. `refreshed_at` stays the last successful
-- refresh; `last_attempted_at` is the newest applied attempt, successful or
-- not, and `refresh_error` is the typed failure of that attempt, NULL after a
-- success. Only a closed code is stored: no message, stderr or response body.
-- Existing rows keep every value and gain NULL status (never attempted).
ALTER TABLE pull_requests ADD COLUMN last_attempted_at INTEGER CHECK(
    last_attempted_at IS NULL OR (
        typeof(last_attempted_at) = 'integer' AND last_attempted_at >= 0
        AND (refreshed_at IS NULL OR refreshed_at <= last_attempted_at)
    )
);
ALTER TABLE pull_requests ADD COLUMN refresh_error TEXT CHECK(
    refresh_error IS NULL OR (
        last_attempted_at IS NOT NULL
        AND refresh_error IN (
            'unavailable', 'timeout', 'cancelled', 'output_too_large',
            'invalid_response', 'execution_failed', 'not_found',
            'unauthorized', 'rate_limited'
        )
    )
);
