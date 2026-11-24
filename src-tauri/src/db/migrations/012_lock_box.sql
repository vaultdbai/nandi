-- Migration 012: the Lock box (`services::lock_box`, Nandi 0.3).
--
-- One row per key: its rules and when it was last used. Never its value:
-- that is in the OS keychain only, under `nandi.lockbox.<name>`. The key
-- is on this computer only, so its rules are too (not the gold copy).
--
-- `kind`: `apiKey`, `password` or `token`. `hosts` and `agents`: JSON
-- arrays of the bound sites and the allowed agents' IDs. `lock`: `open`,
-- `ask` (the default) or `locked`. `unlock_minutes`: how long an OS
-- unlock lasts. `header_name` / `header_prefix`: the request header the
-- egress proxy puts the key in (NULL: the proxy never adds it).
-- `last_used_at` / `last_used_by` / `last_used_host`: the last release.
-- Times are epoch ms.

CREATE TABLE lock_box_keys (
    name           TEXT PRIMARY KEY,
    kind           TEXT NOT NULL,
    hosts          TEXT NOT NULL DEFAULT '[]',
    agents         TEXT NOT NULL DEFAULT '[]',
    lock           TEXT NOT NULL DEFAULT 'ask',
    unlock_minutes INTEGER NOT NULL DEFAULT 15,
    header_name    TEXT,
    header_prefix  TEXT NOT NULL DEFAULT '',
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    last_used_at   INTEGER,
    last_used_by   TEXT,
    last_used_host TEXT
);
