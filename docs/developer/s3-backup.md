# S3 Backup

The built-in `s3_backup` connector backs up the folders a user assigns to their own S3 bucket as Glacier Deep Archive, and can set up the AWS side. The **Personal Backup** pack (`personal-backup`, repo `nandi-personal-backup`) adds the agent that runs it and a setup workflow. It is the first personal pack: agents that look after the user's own things on the user's own accounts.

## Pieces

| Piece                          | Where                                                                                              |
| ------------------------------ | -------------------------------------------------------------------------------------------------- |
| Connector, tools, AWS CLI host | `src-tauri/src/services/connectors/s3_backup.rs` (`S3BackupConnector`, `BackupHost`, `SystemHost`) |
| Triggers from Setup            | `src-tauri/src/services/connectors/backup_triggers.rs`, called by `configure_connector`            |
| Catalog entry and defaults     | `catalog.rs` (`S3_BACKUP`, `declaration`)                                                          |
| Setup UI                       | `src/components/library/ConnectorSetup.tsx` (`ConfigFields`, `SharedFoldersField backup`)          |
| Agent and workflow             | `nandi-personal-backup/agents/backup-keeper/`, `workflows/set-up-s3-backup.yaml`                   |

## Config

```yaml
- id: s3_backup
  kind: s3_backup
  config:
    profile: default # setup: creates the bucket and user
    upload_profile: nandi-backup # backups: upload only
    region: eu-west-1
    bucket: nandi-backup-ada-k3x9q2
    folders:
      - { id: photos, path: ~/Pictures }
```

`folders` uses the shared-folder format (`connectors::folders`); `access` is ignored (backups only read).

## Design

- **Host only.** Every call is the AWS CLI on the host, like the `aws` connector, so no credential reaches a container (see [egress.md](./egress.md)). `BackupHost` is the seam tests fake.
- **Two profiles.** Setup tools use `profile`; backups use `upload_profile`. `create_upload_user` writes the new key with `aws configure --profile <upload_profile>` on stdin (`harness::aws_auth::configure_answers`), so the secret never reaches the model, a command line or a log.
- **Upload-only policy** (`upload_policy`): `s3:ListBucket`, `GetBucketLocation`, `ListBucketMultipartUploads` on the bucket and `PutObject`, `AbortMultipartUpload`, `ListMultipartUploadParts` on its objects. No read and no delete, so a leaked key can't read or wipe backups.
- **Cheapest storage.** `aws s3 sync … --storage-class DEEP_ARCHIVE`, never `--delete`. The bucket blocks public access and aborts unfinished multipart uploads after 7 days. Versioning stays off: Deep Archive bills 180 days minimum per object, so versions would multiply the cost of files that change.
- **Approvals.** `open_aws_signup`, `create_backup_bucket` and `create_upload_user` are sensitive tools; backups and status aren't.
- **Triggers are derived.** Agents can't create triggers and packs can't ship them, and a folder trigger needs the user's own path anyway. So saving the connector's Setup writes them: `s3-backup-<folder>` (file, recursive, create/modify, 10 s debounce, `rate_limit_seconds: 900`) and `s3-backup-nightly` (cron `0 2 * * *`), all targeting `backup-keeper`, and removes `s3-backup-*` triggers of folders that are gone. They are ordinary trigger files, so the user can pause or edit them; a paused one stays paused when Setup is saved again. Nothing is written until the Backup Keeper agent is installed.
- **Bursts.** Dispatch checks `rate_limit_seconds` before the storm guard (`triggers/dispatch.rs`), so a folder of files copied at once is one run plus rate-limited fires, not a storm that disables the trigger. Changes after the allowed fire are caught by the next one or the nightly run.

## Known limits

- Each backup is an agent turn (there's no tool step type), so it needs a model; the turn is one tool call.
- Saving Setup before installing the pack writes no triggers at first; installing any pack afterwards writes them (`connectors::resync_backup_triggers`).
- The pack isn't in the registry yet: sign and publish it as in [marketplace.md](./marketplace.md), and add it to `PACK_REPOS` / `SEED` in `nandi-registry/tools/build_index.py`.

## Tests

See CP-27 in `docs/testing/library-capabilities-studio.md`.
