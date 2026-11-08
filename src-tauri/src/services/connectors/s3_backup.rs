//! The `s3_backup` connector: backs up the folders the user assigns to it
//! into an S3 bucket, stored as Glacier Deep Archive (the cheapest storage
//! class: about $1 per TB a month, restores take hours). It can also set
//! that bucket up: open the AWS sign-up page, create the bucket, and create
//! an IAM user that may only upload to it.
//!
//! Everything runs on the host through the AWS CLI, so no credential ever
//! reaches an agent's container. Setup calls (`create_backup_bucket`,
//! `create_upload_user`) use the admin `profile`; backups use the narrow
//! `upload_profile`, which `create_upload_user` writes with `aws configure`
//! (the new secret goes to the CLI on stdin, never to the model, a command
//! line or a log). Opening a browser and the two setup calls wait for the
//! user's approval.
//!
//! ```yaml
//! - id: s3_backup
//!   kind: s3_backup
//!   config:
//!     profile: default            # setup (admin) profile
//!     upload_profile: nandi-backup
//!     region: us-east-1
//!     bucket: my-nandi-backup
//!     folders:
//!       - {id: photos, path: ~/Pictures}
//! ```
//!
//! A folder `<id>` goes to `s3://<bucket>/<id>/`. Backups only add and
//! replace objects (`aws s3 sync` without `--delete`), so deleting a file on
//! this computer never deletes its backup.

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{json, Value};

use super::aws::{cli_on_path, explain_error};
use super::catalog::config_str;
use super::folders::{self, SharedFolder};
use super::{expand_home, Connector, ConnectorDecl, ToolDef};
use crate::services::harness::aws_auth::{configure_answers, valid_region, KeyProfileInput};

/// The cheapest S3 storage class
pub const STORAGE_CLASS: &str = "DEEP_ARCHIVE";
pub const DEFAULT_UPLOAD_PROFILE: &str = "nandi-backup";
const DEFAULT_USER: &str = "nandi-backup";
const POLICY_NAME: &str = "nandi-backup-upload";
pub const SIGNUP_URL: &str = "https://portal.aws.amazon.com/billing/signup";
/// A setup call (create a bucket, a user, a key) answers within this
const SETUP_TIMEOUT: Duration = Duration::from_secs(60);
/// A folder's upload may take this long
const SYNC_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);
/// Uploaded files listed back to the model at most
const MAX_LISTED: usize = 20;
/// Objects summarized by `list_backups` at most
const MAX_OBJECTS: usize = 1_000;
/// Files that are never worth keeping
const EXCLUDES: &[&str] = &["*.tmp", "~$*", ".DS_Store", "Thumbs.db", "desktop.ini"];

/// One AWS CLI call. No `Debug`: `stdin` may hold a secret.
pub struct CliCall {
    pub args: Vec<String>,
    pub profile: Option<String>,
    pub region: Option<String>,
    pub stdin: Option<String>,
    pub timeout: Duration,
}

/// What the connector needs from the host (tests fake it)
pub trait BackupHost: Send + Sync {
    /// Runs `aws <args>` and returns its standard output
    fn aws(&self, call: CliCall) -> BoxFuture<'_, Result<String, String>>;
    /// Opens `url` in the default browser
    fn open_url(&self, url: &str) -> Result<(), String>;
}

/// The real AWS CLI and browser
pub struct SystemHost;

impl BackupHost for SystemHost {
    fn aws(&self, call: CliCall) -> BoxFuture<'_, Result<String, String>> {
        Box::pin(async move {
            use tokio::io::AsyncWriteExt;

            let mut command = tokio::process::Command::new("aws");
            command.args(&call.args);
            if let Some(profile) = &call.profile {
                command.args(["--profile", profile]);
            }
            if let Some(region) = &call.region {
                command.args(["--region", region]);
            }
            command
                .stdin(if call.stdin.is_some() {
                    std::process::Stdio::piped()
                } else {
                    std::process::Stdio::null()
                })
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            #[cfg(windows)]
            {
                // No console window flashing up for aws.exe
                command.creation_flags(0x0800_0000);
            }
            let mut child = command.spawn().map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => {
                    "The AWS CLI isn't installed: install it, then try again".to_string()
                }
                _ => format!("Couldn't run the AWS CLI: {e}"),
            })?;
            if let (Some(input), Some(mut stdin)) = (call.stdin, child.stdin.take()) {
                stdin
                    .write_all(input.as_bytes())
                    .await
                    .map_err(|e| format!("Couldn't answer the AWS CLI: {e}"))?;
            }
            let minutes = call.timeout.as_secs() / 60;
            let output = tokio::time::timeout(call.timeout, child.wait_with_output())
                .await
                .map_err(|_| {
                    if minutes > 0 {
                        format!("The AWS CLI didn't finish within {minutes} minutes")
                    } else {
                        "The AWS CLI didn't answer in time".to_string()
                    }
                })?
                .map_err(|e| format!("Couldn't run the AWS CLI: {e}"))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(explain_error(stderr.trim()));
            }
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        })
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        tauri_plugin_opener::open_url(url, None::<&str>)
            .map_err(|e| format!("Couldn't open the browser: {e}"))
    }
}

/// The connector's settings (`config` in `.nandi/connectors.yaml`)
#[derive(Debug, Clone, PartialEq)]
pub struct BackupSettings {
    /// For setup; `None` is the CLI's default profile
    pub profile: Option<String>,
    pub upload_profile: String,
    pub region: Option<String>,
    pub bucket: Option<String>,
    pub folders: Vec<SharedFolder>,
}

impl BackupSettings {
    pub fn from_decl(decl: &ConnectorDecl) -> Result<Self, String> {
        Ok(Self {
            profile: config_str(decl, "profile"),
            upload_profile: config_str(decl, "upload_profile")
                .unwrap_or_else(|| DEFAULT_UPLOAD_PROFILE.to_string()),
            region: config_str(decl, "region"),
            bucket: config_str(decl, "bucket"),
            folders: folders::shared_folders(decl)?,
        })
    }
}

/// S3's bucket naming rules (3–63 characters: lowercase letters, digits,
/// '-' and '.', starting and ending with a letter or digit)
pub fn valid_bucket(name: &str) -> Result<&str, String> {
    let name = name.trim();
    let edge = |c: Option<char>| c.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let ok = (3..=63).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
        && edge(name.chars().next())
        && edge(name.chars().last())
        && !name.contains("..");
    if ok {
        Ok(name)
    } else {
        Err(format!(
            "'{name}' isn't a valid bucket name: 3–63 lowercase letters, digits, '-' or '.'"
        ))
    }
}

fn valid_user(name: &str) -> Result<&str, String> {
    let name = name.trim();
    let ok = (1..=64).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+=,.@_-".contains(c));
    if ok {
        Ok(name)
    } else {
        Err(format!("'{name}' isn't a valid IAM user name"))
    }
}

/// Upload only: list the bucket and write objects. No read and no delete,
/// so a leaked key can't read the backups or wipe them.
pub fn upload_policy(bucket: &str) -> Value {
    json!({
        "Version": "2012-10-17",
        "Statement": [
            {
                "Sid": "ListTheBackupBucket",
                "Effect": "Allow",
                "Action": ["s3:ListBucket", "s3:GetBucketLocation", "s3:ListBucketMultipartUploads"],
                "Resource": format!("arn:aws:s3:::{bucket}")
            },
            {
                "Sid": "UploadBackups",
                "Effect": "Allow",
                "Action": ["s3:PutObject", "s3:AbortMultipartUpload", "s3:ListMultipartUploadParts"],
                "Resource": format!("arn:aws:s3:::{bucket}/*")
            }
        ]
    })
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn json_out(stdout: &str) -> Result<Value, String> {
    if stdout.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(stdout).map_err(|e| format!("Unexpected output from the AWS CLI: {e}"))
}

/// The files `aws s3 sync` reports as uploaded (`upload: <file> to s3://…`)
fn uploaded_files(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|line| line.trim().strip_prefix("upload: "))
        .map(|rest| rest.split(" to s3://").next().unwrap_or(rest).to_string())
        .collect()
}

pub struct S3BackupConnector {
    host: Arc<dyn BackupHost>,
    settings: BackupSettings,
}

impl S3BackupConnector {
    pub fn new(host: Arc<dyn BackupHost>, settings: BackupSettings) -> Self {
        Self { host, settings }
    }

    pub fn tool_defs() -> Vec<ToolDef> {
        let tool = |name: &str, description: &str, schema: Value, sensitive: bool| ToolDef {
            name: name.into(),
            description: description.into(),
            input_schema: schema,
            sensitive,
        };
        let bucket =
            json!({"type": "string", "description": "the bucket (default: the one in Setup)"});
        vec![
            tool(
                "status",
                "What's set up: whether the setup and upload profiles can sign in to AWS (and which account), the bucket, region, and the folders assigned for backup with their paths.",
                json!({"type": "object", "properties": {}}),
                false,
            ),
            tool(
                "open_aws_signup",
                "Opens the AWS sign-up page in the user's browser, to create an AWS account. The user approves it first.",
                json!({"type": "object", "properties": {}}),
                true,
            ),
            tool(
                "create_backup_bucket",
                "Creates a private S3 bucket for backups with the setup profile: public access blocked, unfinished uploads cleaned up after 7 days. Backups go in as Glacier Deep Archive. The user approves it first.",
                json!({"type": "object", "properties": {
                    "bucket": {"type": "string", "description": "a globally unique name, e.g. nandi-backup-<name>-<random>"},
                    "region": {"type": "string", "description": "e.g. us-east-1 (default: the one in Setup)"}
                }, "required": ["bucket"]}),
                true,
            ),
            tool(
                "create_upload_user",
                "Creates an IAM user that may only upload to the bucket (no read, no delete), creates its access key, and saves it as the upload profile on this computer. The key is never shown. The user approves it first.",
                json!({"type": "object", "properties": {
                    "bucket": bucket,
                    "user_name": {"type": "string", "description": "default nandi-backup"}
                }}),
                true,
            ),
            tool(
                "backup_folder",
                "Uploads new and changed files of one assigned folder to s3://<bucket>/<folder>/ as Glacier Deep Archive, with the upload profile. Never deletes backups.",
                json!({"type": "object", "properties": {
                    "folder": {"type": "string", "description": "the folder's ID from status"},
                    "bucket": bucket
                }, "required": ["folder"]}),
                false,
            ),
            tool(
                "backup_all",
                "backup_folder for every assigned folder.",
                json!({"type": "object", "properties": {"bucket": bucket}}),
                false,
            ),
            tool(
                "list_backups",
                "How many files a folder (or the whole bucket) has backed up, their total size, and the latest upload.",
                json!({"type": "object", "properties": {
                    "folder": {"type": "string"},
                    "bucket": bucket
                }}),
                false,
            ),
        ]
    }

    fn bucket(&self, args: &Value) -> Result<String, String> {
        let bucket = str_arg(args, "bucket")
            .map(str::to_string)
            .or_else(|| self.settings.bucket.clone())
            .ok_or("No bucket yet: create one with create_backup_bucket, then set it in Library › S3 Backup › Setup")?;
        Ok(valid_bucket(&bucket)?.to_string())
    }

    fn region(&self, args: &Value) -> Result<Option<String>, String> {
        match str_arg(args, "region").or(self.settings.region.as_deref()) {
            Some(region) => Ok(Some(valid_region(region)?.to_string())),
            None => Ok(None),
        }
    }

    fn setup_call(&self, args: &[&str], region: Option<String>) -> CliCall {
        CliCall {
            args: args.iter().map(|a| (*a).to_string()).collect(),
            profile: self.settings.profile.clone(),
            region,
            stdin: None,
            timeout: SETUP_TIMEOUT,
        }
    }

    /// The AWS account `profile` signs in to, or why it can't
    async fn whoami(&self, profile: Option<String>) -> Value {
        let call = CliCall {
            args: ["sts", "get-caller-identity", "--output", "json"]
                .map(str::to_string)
                .to_vec(),
            profile,
            region: self.settings.region.clone(),
            stdin: None,
            timeout: SETUP_TIMEOUT,
        };
        match self.host.aws(call).await.and_then(|out| json_out(&out)) {
            Ok(identity) => json!({"account": identity["Account"], "arn": identity["Arn"]}),
            Err(error) => json!({"error": error}),
        }
    }

    async fn status(&self) -> Value {
        json!({
            "setup_sign_in": self.whoami(self.settings.profile.clone()).await,
            "upload_sign_in": self.whoami(Some(self.settings.upload_profile.clone())).await,
            "bucket": self.settings.bucket,
            "region": self.settings.region,
            "setup_profile": self.settings.profile.as_deref().unwrap_or("default"),
            "upload_profile": self.settings.upload_profile,
            "storage_class": STORAGE_CLASS,
            "folders": self.settings.folders.iter().map(|f| json!({
                "id": f.id,
                "path": f.path,
                "destination": self.settings.bucket.as_ref().map(|b| format!("s3://{b}/{}/", f.id)),
            })).collect::<Vec<_>>(),
        })
    }

    async fn create_bucket(&self, args: &Value) -> Result<Value, String> {
        let bucket = valid_bucket(str_arg(args, "bucket").ok_or("Missing 'bucket'")?)?.to_string();
        let region = self
            .region(args)?
            .unwrap_or_else(|| "us-east-1".to_string());
        let constraint = format!("LocationConstraint={region}");
        let mut create = vec!["s3api", "create-bucket", "--bucket", &bucket];
        // us-east-1 takes no location constraint
        if region != "us-east-1" {
            create.extend(["--create-bucket-configuration", &constraint]);
        }
        self.host
            .aws(self.setup_call(&create, Some(region.clone())))
            .await?;
        self.host
            .aws(self.setup_call(
                &[
                    "s3api",
                    "put-public-access-block",
                    "--bucket",
                    &bucket,
                    "--public-access-block-configuration",
                    "BlockPublicAcls=true,IgnorePublicAcls=true,BlockPublicPolicy=true,RestrictPublicBuckets=true",
                ],
                Some(region.clone()),
            ))
            .await?;
        let lifecycle = json!({"Rules": [{
            "ID": "abort-unfinished-uploads",
            "Status": "Enabled",
            "Filter": {},
            "AbortIncompleteMultipartUpload": {"DaysAfterInitiation": 7}
        }]})
        .to_string();
        self.host
            .aws(self.setup_call(
                &[
                    "s3api",
                    "put-bucket-lifecycle-configuration",
                    "--bucket",
                    &bucket,
                    "--lifecycle-configuration",
                    &lifecycle,
                ],
                Some(region.clone()),
            ))
            .await?;
        log::info!("Created the backup bucket '{bucket}' in {region}");
        Ok(json!({
            "bucket": bucket,
            "region": region,
            "storage_class": STORAGE_CLASS,
            "next": "Set this bucket and region in Library › S3 Backup › Setup, then create the upload user."
        }))
    }

    async fn create_user(&self, args: &Value) -> Result<Value, String> {
        let bucket = self.bucket(args)?;
        let user = valid_user(str_arg(args, "user_name").unwrap_or(DEFAULT_USER))?.to_string();
        let policy = upload_policy(&bucket).to_string();
        self.host
            .aws(self.setup_call(&["iam", "create-user", "--user-name", &user], None))
            .await
            .map_err(|e| {
                if e.contains("EntityAlreadyExists") {
                    format!("The IAM user '{user}' already exists: pass another user_name, or delete it in the AWS console first")
                } else {
                    e
                }
            })?;
        self.host
            .aws(self.setup_call(
                &[
                    "iam",
                    "put-user-policy",
                    "--user-name",
                    &user,
                    "--policy-name",
                    POLICY_NAME,
                    "--policy-document",
                    &policy,
                ],
                None,
            ))
            .await?;
        let mut key_call =
            self.setup_call(&["iam", "create-access-key", "--user-name", &user], None);
        key_call
            .args
            .extend(["--output".to_string(), "json".to_string()]);
        let key = json_out(&self.host.aws(key_call).await?)?;
        let (Some(id), Some(secret)) = (
            key["AccessKey"]["AccessKeyId"].as_str(),
            key["AccessKey"]["SecretAccessKey"].as_str(),
        ) else {
            return Err("AWS didn't return the new access key".to_string());
        };
        let answers = configure_answers(&KeyProfileInput {
            profile: self.settings.upload_profile.clone(),
            access_key_id: id.to_string(),
            secret_access_key: secret.to_string(),
            region: self.settings.region.clone(),
        })?;
        let profile = self.settings.upload_profile.clone();
        self.host
            .aws(CliCall {
                args: vec!["configure".into(), "--profile".into(), profile.clone()],
                profile: None,
                region: None,
                stdin: Some(answers),
                timeout: SETUP_TIMEOUT,
            })
            .await
            .map_err(|e| {
                format!("The key was created but couldn't be saved ({e}): delete access key {id} of '{user}' in the AWS console and try again")
            })?;
        log::info!("Created the upload-only IAM user '{user}' and saved it as profile '{profile}'");
        Ok(json!({
            "user": user,
            "profile": profile,
            "policy": POLICY_NAME,
            "allows": "list the bucket and upload; no reading and no deleting",
            "access_key_id": id,
        }))
    }

    fn folder(&self, id: &str) -> Result<&SharedFolder, String> {
        self.settings
            .folders
            .iter()
            .find(|f| f.id == id)
            .ok_or_else(|| {
                let ids: Vec<&str> = self
                    .settings
                    .folders
                    .iter()
                    .map(|f| f.id.as_str())
                    .collect();
                if ids.is_empty() {
                    "No folders are assigned for backup: add them in Library › S3 Backup › Setup"
                        .to_string()
                } else {
                    format!(
                        "No backup folder '{id}': the folders are {}",
                        ids.join(", ")
                    )
                }
            })
    }

    async fn backup(&self, folder: &SharedFolder, bucket: &str) -> Result<Value, String> {
        let path = expand_home(&folder.path);
        if !path.is_dir() {
            return Err(format!(
                "The backup folder '{}' ({}) isn't there",
                folder.id,
                path.display()
            ));
        }
        let destination = format!("s3://{bucket}/{}/", folder.id);
        let mut args = vec![
            "s3".to_string(),
            "sync".to_string(),
            path.to_string_lossy().into_owned(),
            destination.clone(),
            "--storage-class".to_string(),
            STORAGE_CLASS.to_string(),
            "--no-progress".to_string(),
        ];
        for pattern in EXCLUDES {
            args.extend(["--exclude".to_string(), (*pattern).to_string()]);
        }
        // Agents' screenshots stay on this computer (kept 30 days)
        if let Some(pattern) = crate::utils::nandi_config::work_dir()
            .ok()
            .and_then(|work| crate::services::run_plan::screenshots::backup_exclude(&path, &work))
        {
            args.extend(["--exclude".to_string(), pattern]);
        }
        let stdout = self
            .host
            .aws(CliCall {
                args,
                profile: Some(self.settings.upload_profile.clone()),
                region: self.settings.region.clone(),
                stdin: None,
                timeout: SYNC_TIMEOUT,
            })
            .await?;
        let files = uploaded_files(&stdout);
        log::info!(
            "Backed up {} file(s) of '{}' to {destination}",
            files.len(),
            folder.id
        );
        Ok(json!({
            "folder": folder.id,
            "destination": destination,
            "uploaded": files.len(),
            "files": files.iter().take(MAX_LISTED).collect::<Vec<_>>(),
        }))
    }

    async fn list(&self, args: &Value) -> Result<Value, String> {
        let bucket = self.bucket(args)?;
        let prefix = match str_arg(args, "folder") {
            Some(id) => format!("{}/", self.folder(id)?.id),
            None => String::new(),
        };
        let max = MAX_OBJECTS.to_string();
        let mut call_args = vec![
            "s3api",
            "list-objects-v2",
            "--bucket",
            &bucket,
            "--max-items",
            &max,
            "--output",
            "json",
        ];
        if !prefix.is_empty() {
            call_args.extend(["--prefix", &prefix]);
        }
        let out = json_out(
            &self
                .host
                .aws(CliCall {
                    args: call_args.iter().map(|a| (*a).to_string()).collect(),
                    profile: Some(self.settings.upload_profile.clone()),
                    region: self.settings.region.clone(),
                    stdin: None,
                    timeout: SETUP_TIMEOUT,
                })
                .await?,
        )?;
        let objects = out["Contents"].as_array().cloned().unwrap_or_default();
        let bytes: u64 = objects.iter().filter_map(|o| o["Size"].as_u64()).sum();
        let latest = objects
            .iter()
            .filter_map(|o| o["LastModified"].as_str())
            .max()
            .map(str::to_string);
        Ok(json!({
            "bucket": bucket,
            "prefix": prefix,
            "files": objects.len(),
            "bytes": bytes,
            "latest_upload": latest,
            "more": out.get("NextToken").is_some(),
        }))
    }

    async fn run(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "status" => Ok(self.status().await),
            "open_aws_signup" => {
                self.host.open_url(SIGNUP_URL)?;
                Ok(json!({"opened": SIGNUP_URL}))
            }
            "create_backup_bucket" => self.create_bucket(args).await,
            "create_upload_user" => self.create_user(args).await,
            "backup_folder" => {
                let bucket = self.bucket(args)?;
                let id = str_arg(args, "folder").ok_or("Missing 'folder'")?;
                self.backup(self.folder(id)?, &bucket).await
            }
            "backup_all" => {
                let bucket = self.bucket(args)?;
                if self.settings.folders.is_empty() {
                    return Err(
                        "No folders are assigned for backup: add them in Library › S3 Backup › Setup"
                            .to_string(),
                    );
                }
                let mut results = Vec::new();
                for folder in &self.settings.folders {
                    results.push(match self.backup(folder, &bucket).await {
                        Ok(result) => result,
                        Err(error) => json!({"folder": folder.id, "error": error}),
                    });
                }
                Ok(json!({"folders": results}))
            }
            "list_backups" => self.list(args).await,
            other => Err(format!("Unknown tool '{other}'")),
        }
    }
}

impl Connector for S3BackupConnector {
    fn list_tools(&self) -> Vec<ToolDef> {
        Self::tool_defs()
    }

    fn call_tool<'a>(
        &'a self,
        name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(self.run(name, args))
    }

    fn health_check(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            if cli_on_path() {
                Ok(())
            } else {
                Err("The AWS CLI isn't installed: install it, then start S3 Backup again".into())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A recorded call: its arguments, profile, and whether it had stdin
    type Call = (Vec<String>, Option<String>, bool);

    /// Answers by the CLI's first two words, and records every call
    #[derive(Default)]
    struct Fake {
        answers: Vec<(String, Result<String, String>)>,
        calls: Mutex<Vec<Call>>,
        opened: Mutex<Vec<String>>,
    }

    impl Fake {
        fn with(answers: &[(&str, Result<&str, &str>)]) -> Arc<Self> {
            Arc::new(Self {
                answers: answers
                    .iter()
                    .map(|(k, v)| {
                        (
                            (*k).to_string(),
                            v.map(str::to_string).map_err(str::to_string),
                        )
                    })
                    .collect(),
                ..Self::default()
            })
        }

        fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl BackupHost for Fake {
        fn aws(&self, call: CliCall) -> BoxFuture<'_, Result<String, String>> {
            let key = call
                .args
                .iter()
                .take(2)
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            self.calls
                .lock()
                .unwrap()
                .push((call.args, call.profile, call.stdin.is_some()));
            let answer = self
                .answers
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or(Ok(String::new()));
            Box::pin(async move { answer })
        }

        fn open_url(&self, url: &str) -> Result<(), String> {
            self.opened.lock().unwrap().push(url.to_string());
            Ok(())
        }
    }

    fn settings(folder: &Path) -> BackupSettings {
        BackupSettings {
            profile: Some("admin".into()),
            upload_profile: "nandi-backup".into(),
            region: Some("eu-west-1".into()),
            bucket: Some("my-backups".into()),
            folders: vec![SharedFolder {
                id: "photos".into(),
                path: folder.to_string_lossy().into_owned(),
                access: folders::Access::Read,
            }],
        }
    }

    use std::path::Path;

    #[tokio::test]
    async fn a_folder_syncs_as_deep_archive_with_the_upload_profile() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake::with(&[(
            "s3 sync",
            Ok("upload: C:\\pics\\a.jpg to s3://my-backups/photos/a.jpg\nupload: b.jpg to s3://my-backups/photos/b.jpg\n"),
        )]);
        let connector = S3BackupConnector::new(fake.clone(), settings(dir.path()));
        let out = connector
            .call_tool("backup_folder", &json!({"folder": "photos"}))
            .await
            .unwrap();
        assert_eq!(out["uploaded"], 2);
        assert_eq!(out["destination"], "s3://my-backups/photos/");
        let (args, profile, _) = &fake.calls()[0];
        assert_eq!(profile.as_deref(), Some("nandi-backup"));
        assert!(args
            .windows(2)
            .any(|w| w == ["--storage-class", "DEEP_ARCHIVE"]));
        assert!(
            !args.iter().any(|a| a == "--delete"),
            "never deletes backups"
        );
    }

    #[tokio::test]
    async fn only_assigned_folders_are_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let connector = S3BackupConnector::new(Fake::with(&[]), settings(dir.path()));
        let error = connector
            .call_tool("backup_folder", &json!({"folder": "secrets"}))
            .await
            .unwrap_err();
        assert!(error.contains("the folders are photos"), "{error}");
    }

    #[tokio::test]
    async fn a_bucket_is_private_and_cleans_up_unfinished_uploads() {
        let fake = Fake::with(&[]);
        let dir = tempfile::tempdir().unwrap();
        let connector = S3BackupConnector::new(fake.clone(), settings(dir.path()));
        connector
            .call_tool(
                "create_backup_bucket",
                &json!({"bucket": "nandi-backup-ada"}),
            )
            .await
            .unwrap();
        let calls = fake.calls();
        assert_eq!(calls.len(), 3);
        assert!(calls[0]
            .0
            .iter()
            .any(|a| a == "LocationConstraint=eu-west-1"));
        assert_eq!(calls[0].1.as_deref(), Some("admin"));
        assert!(calls[1]
            .0
            .iter()
            .any(|a| a.contains("BlockPublicPolicy=true")));
        assert!(calls[2]
            .0
            .iter()
            .any(|a| a.contains("AbortIncompleteMultipartUpload")));
    }

    #[tokio::test]
    async fn the_upload_user_key_goes_to_the_profile_never_the_model() {
        let fake = Fake::with(&[(
            "iam create-access-key",
            Ok(
                r#"{"AccessKey": {"AccessKeyId": "AKIAEXAMPLEKEY12345", "SecretAccessKey": "s3cr3t/value"}}"#,
            ),
        )]);
        let dir = tempfile::tempdir().unwrap();
        let connector = S3BackupConnector::new(fake.clone(), settings(dir.path()));
        let out = connector
            .call_tool("create_upload_user", &json!({}))
            .await
            .unwrap();
        assert!(!out.to_string().contains("s3cr3t"), "{out}");
        assert_eq!(out["profile"], "nandi-backup");
        let calls = fake.calls();
        let policy = calls[1].0.last().unwrap();
        assert!(policy.contains("s3:PutObject") && !policy.contains("s3:GetObject"));
        assert!(!policy.contains("DeleteObject"));
        let (configure, profile, stdin) = calls.last().unwrap();
        assert_eq!(configure[..3], ["configure", "--profile", "nandi-backup"]);
        assert!(profile.is_none() && *stdin, "the secret goes on stdin");
        assert!(!configure.iter().any(|a| a.contains("s3cr3t")));
    }

    #[tokio::test]
    async fn opening_the_signup_page_uses_the_browser() {
        let fake = Fake::with(&[]);
        let dir = tempfile::tempdir().unwrap();
        let connector = S3BackupConnector::new(fake.clone(), settings(dir.path()));
        connector
            .call_tool("open_aws_signup", &json!({}))
            .await
            .unwrap();
        assert_eq!(*fake.opened.lock().unwrap(), [SIGNUP_URL]);
    }

    #[test]
    fn bucket_names_follow_s3_rules() {
        assert!(valid_bucket("nandi-backup-ada").is_ok());
        assert!(valid_bucket("ab").is_err());
        assert!(valid_bucket("Upper").is_err());
        assert!(valid_bucket("-edge").is_err());
        assert!(valid_bucket("a..b").is_err());
    }

    #[tokio::test]
    async fn listing_sums_the_backed_up_files() {
        let fake = Fake::with(&[(
            "s3api list-objects-v2",
            Ok(
                r#"{"Contents": [{"Key": "photos/a", "Size": 10, "LastModified": "2026-10-01T00:00:00Z"}, {"Key": "photos/b", "Size": 5, "LastModified": "2026-10-03T00:00:00Z"}]}"#,
            ),
        )]);
        let dir = tempfile::tempdir().unwrap();
        let connector = S3BackupConnector::new(fake, settings(dir.path()));
        let out = connector
            .call_tool("list_backups", &json!({"folder": "photos"}))
            .await
            .unwrap();
        assert_eq!(out["files"], 2);
        assert_eq!(out["bytes"], 15);
        assert_eq!(out["latest_upload"], "2026-10-03T00:00:00Z");
    }
}
