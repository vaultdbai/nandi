# Task: Snapshots of Your Nandi in Your Own Drive

**From:** owner 2026-11-27: "syncing my notes and state.db to primary drive of connected email, for example onedrive or google drive".
**Serves:** state (nothing a person has is lost with a laptop) and trust (encrypted before it leaves, the narrowest drive scope).
**Size:** M, in three slices that each land on their own. Not the current goal: waits its turn.
**Related:** `services/connectors/s3_backup.rs` (the S3 backup, same idea for folders), `db/migrations.rs` (`state.db.bak-v<n>` copies), the email sign-ins (`services/email/sign_ins.rs`, `outlook.rs`, `gmail.rs`).

## Decisions

- **Snapshots, never a live sync of `state.db`.** A drive client syncing a live SQLite file uploads half-written pages and misses the WAL. Two laptops writing to it make conflicts no one can merge. Nandi takes a consistent copy with SQLite's backup (`VACUUM INTO` or the backup API), then uploads that one file.
- **Encrypted on this computer first.** `state.db` holds email-derived to-dos and their context, run records and the audit trail. The snapshot is sealed with a key kept in the OS keyring (and shown once as a recovery phrase), so the drive holds only ciphertext.
- **The app folder only.** OneDrive `Files.ReadWrite.AppFolder`, Google `drive.appdata`. Never full Drive access. It is a separate consent from mail, asked when the person turns snapshots on.
- **The gold copy goes as files.** `~/.nandi/repo/.nandi/` (agents, workflows, notes) is plain files already: upload them as they are, in the same app folder. No secrets: files hold `secret_refs` only.
- **Works without the relay.** It is the person's own drive, through their own sign-in.

## Slices

| #   | Slice                                                                                                                                                        | Done when                                                                                      |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------- |
| 1   | A sealed snapshot on disk: `state.db` copied consistently, sealed with the keyring key, plus the gold copy, in one dated archive. A "Make a snapshot" button | A test restores the archive into an empty folder and the app opens it with every row and file  |
| 2   | Upload to the app folder of the connected Outlook or Gmail account. Keep the last N. A daily schedule, recorded in Decisions                                 | Turning it on asks for the app-folder scope only; a run record says which snapshot went where  |
| 3   | Restore on a new computer: sign in, pick a snapshot, type the recovery phrase                                                                                | A fresh install comes back with its notes, to-dos, runs and agents; migrations run on the copy |

## Open Questions

- Snapshot schedule: daily, or after a number of changes?
- How many snapshots to keep (7? 30?), and how big a `state.db` gets in a year of use.
- Restoring onto a computer that already has data: replace, or refuse?
- Does the same sealed archive also go to S3 for Backup Keeper users?
