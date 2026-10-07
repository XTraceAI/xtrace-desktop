-- A pull request whose check failed in a refresh the user started by hand.
-- `manual_failed_at` is that attempt's time. It stays through later automatic
-- failures and is cleared by the next success, so the Dashboard can stop
-- asking the user about a pull request they already tried and that may never
-- be checkable (a deleted repository, another account's). Additive only:
-- existing rows gain NULL (no manual failure recorded).
ALTER TABLE pull_requests ADD COLUMN manual_failed_at INTEGER CHECK(
    manual_failed_at IS NULL OR (
        typeof(manual_failed_at) = 'integer' AND manual_failed_at >= 0
        AND refresh_error IS NOT NULL
        AND last_attempted_at IS NOT NULL
        AND manual_failed_at <= last_attempted_at
    )
);
