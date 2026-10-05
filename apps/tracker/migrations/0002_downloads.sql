-- I5: per-installer download counters (website convenience, not part of the
-- msp-v1 tracker contract). Counted redirects live at /api/download/<file>;
-- file names are whitelisted-shaped and must exist under public/downloads.
CREATE TABLE IF NOT EXISTS download_counts (
    file    TEXT PRIMARY KEY,
    count   BIGINT NOT NULL DEFAULT 0,
    last_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
