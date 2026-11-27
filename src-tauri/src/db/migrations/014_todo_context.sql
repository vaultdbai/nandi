-- Migration 014: what a to-do was read from (`domain::todo::TodoContext`,
-- Nandi 0.3.3).
--
-- `context`: JSON of who sent it, its subject or title, when, the account it
-- came to, the lines that ask for it and why it matters, as Nandi saw them
-- when it added the to-do. Empty for work a person assigned, a to-do added
-- by hand, and every to-do added before 0.3.3.

ALTER TABLE gateway_assignments ADD COLUMN context TEXT;
