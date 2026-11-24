-- Migration 011: an agent's file is `agent.md` (Nandi 0.3).
--
-- The gold copy's upgrade renames each agent folder's `skill.md` to
-- `agent.md` (`services::repos::from_0_2`). The weekly retro's proposals
-- name that file twice: `target` `skill` is now `agent`, and `path` ends
-- in `agent.md`. Playbooks (`skills/<name>.md`) keep their paths.

UPDATE retro_proposals SET target = 'agent' WHERE target = 'skill';

UPDATE retro_proposals
SET path = substr(path, 1, length(path) - length('skill.md')) || 'agent.md'
WHERE path LIKE '%/skill.md' AND path NOT LIKE '%/skills/%';
