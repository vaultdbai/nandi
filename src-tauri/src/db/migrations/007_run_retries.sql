-- Migration 007: Nandi tries its own failed work again
-- (`services::run_retry`). A retry is a new run in the same conversation
-- with the same input; it names the failed run it tries again.
--
-- Every existing run is a first attempt: `retry_of` NULL, `attempt` 1.

ALTER TABLE runs ADD COLUMN retry_of TEXT;
ALTER TABLE runs ADD COLUMN attempt INTEGER NOT NULL DEFAULT 1;
CREATE INDEX runs_retry_of ON runs(retry_of);
