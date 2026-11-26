-- Migration 013: how much a paired phone may ask (`own_devices::controls`,
-- Nandi 0.3.2).
--
-- `ask_level`: `answer` (read and answer, no actions), `confirm` (act, every
-- action waits for the person's yes: the default) or `off` (the phone only
-- approves). The device is on this computer only, so its level is too.

ALTER TABLE own_devices ADD COLUMN ask_level TEXT NOT NULL DEFAULT 'confirm';
