-- Migration 010: the person's own devices (`services::own_devices`).
--
-- A phone paired with this Nandi through the relay (protocol 8). The
-- relay keeps the device's token; this table keeps what only the Nandi
-- may know: the name the person gave it, the keys they confirmed by
-- fingerprint, the PIN's slow hash, the lockout and the replay cursor.
--
-- `state`: `pairing` (a code is shown), `claimed` (a phone claimed it;
-- the person hasn't confirmed its fingerprint) or `active`. A revoked
-- device's row is deleted.
-- `signing_key` / `sealing_key`: the phone's public keys from its claim
-- (NULL while pairing). `pin_hash`: an Argon2id PHC string, set when the
-- person confirms (NULL before). `failed_pins`: wrong PINs in a row;
-- `locked_at` is set when they reach the limit, and only the desktop
-- unlocks. `last_seq`: the highest envelope `seq` taken from the device;
-- anything at or below it is a replay. Times are epoch ms.

CREATE TABLE own_devices (
    device_id    TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    state        TEXT NOT NULL,
    signing_key  TEXT,
    sealing_key  TEXT,
    pin_hash     TEXT,
    failed_pins  INTEGER NOT NULL DEFAULT 0,
    locked_at    INTEGER,
    last_seq     INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL,
    expires_at   INTEGER,
    paired_at    INTEGER,
    last_seen_at INTEGER
);
