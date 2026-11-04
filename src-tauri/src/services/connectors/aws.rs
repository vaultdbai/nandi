//! The `aws` connector: operations tools for an ops agent. It covers
//! CloudWatch alarms, metrics and Logs (Insights queries and recent events),
//! AWS Batch jobs, ECS services and EC2 instance status. Restarting a failed
//! job and redeploying an ECS service wait for the user's approval.
//!
//! The calls run on the host through the AWS CLI (`aws … --output json`)
//! under the profile and region set in Setup, so whatever the profile uses
//! (keys, SSO, a `credential_process`) works and refreshes on its own, and
//! no credential ever reaches an agent's container.

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{json, Value};

use super::{Connector, ToolDef};

/// One AWS CLI call must answer within this
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// A Logs Insights query is polled this long before giving up
const QUERY_WAIT: Duration = Duration::from_secs(60);
const QUERY_POLL: Duration = Duration::from_secs(1);
const MAX_MINUTES: u64 = 7 * 24 * 60;
const MAX_ROWS: u64 = 200;
const MAX_MESSAGE_CHARS: usize = 1_000;
/// ECS describe-services takes at most this many services per call
const MAX_SERVICES: usize = 10;
/// The latest events kept per ECS service
const SERVICE_EVENTS: usize = 5;

/// Runs `aws <args> --output json` and returns the parsed JSON (tests fake it)
pub trait AwsRunner: Send + Sync {
    fn run(&self, args: Vec<String>) -> BoxFuture<'_, Result<Value, String>>;
}

/// The real AWS CLI, with the connector's profile and region
pub struct CliRunner {
    profile: Option<String>,
    region: Option<String>,
}

impl CliRunner {
    pub fn new(profile: Option<String>, region: Option<String>) -> Self {
        Self { profile, region }
    }
}

impl AwsRunner for CliRunner {
    fn run(&self, args: Vec<String>) -> BoxFuture<'_, Result<Value, String>> {
        Box::pin(async move {
            let mut command = tokio::process::Command::new("aws");
            command.args(&args).args(["--output", "json"]);
            if let Some(profile) = &self.profile {
                command.args(["--profile", profile]);
            }
            if let Some(region) = &self.region {
                command.args(["--region", region]);
            }
            command
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true);
            #[cfg(windows)]
            {
                // No console window flashing up for aws.exe
                command.creation_flags(0x0800_0000);
            }
            let output = tokio::time::timeout(CALL_TIMEOUT, command.output())
                .await
                .map_err(|_| "The AWS CLI didn't answer within 60 seconds".to_string())?
                .map_err(|e| match e.kind() {
                    std::io::ErrorKind::NotFound => {
                        "The AWS CLI isn't installed: install it, then set the profile in Setup"
                            .to_string()
                    }
                    _ => format!("Couldn't run the AWS CLI: {e}"),
                })?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(explain_error(stderr.trim()));
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.trim().is_empty() {
                return Ok(Value::Null);
            }
            serde_json::from_str(&stdout)
                .map_err(|e| format!("Unexpected output from the AWS CLI: {e}"))
        })
    }
}

/// The CLI's error, with the next step when it's about signing in
pub(super) fn explain_error(stderr: &str) -> String {
    let first = stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(stderr);
    let lower = stderr.to_lowercase();
    if lower.contains("sso") && (lower.contains("expired") || lower.contains("token")) {
        format!("{first}\nThe AWS SSO sign-in expired: run `aws sso login` for this profile, then try again.")
    } else if lower.contains("could not be found") && lower.contains("profile") {
        format!("{first}\nThat AWS profile doesn't exist: choose one from ~/.aws/config in Setup.")
    } else if lower.contains("unable to locate credentials") || lower.contains("expiredtoken") {
        format!("{first}\nThe profile has no working credentials: sign in or refresh them, then try again.")
    } else if lower.contains("accessdenied") || lower.contains("not authorized") {
        format!("{first}\nThe profile isn't allowed to do this: ask for the permission, or use another profile.")
    } else {
        first.to_string()
    }
}

/// Whether an `aws` executable is on PATH (no process started)
pub(super) fn cli_on_path() -> bool {
    let names: &[&str] = if cfg!(windows) {
        &["aws.exe", "aws.cmd", "aws"]
    } else {
        &["aws"]
    };
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| names.iter().any(|n| dir.join(n).is_file()))
    })
}

pub struct AwsConnector {
    runner: Arc<dyn AwsRunner>,
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Missing string argument '{key}'"))
}

fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn num(args: &Value, key: &str, default: u64, max: u64) -> u64 {
    args.get(key)
        .and_then(Value::as_u64)
        .unwrap_or(default)
        .clamp(1, max)
}

fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Epoch ms (or s) as UTC, e.g. `2026-10-24 22:13 UTC`
fn when(value: &Value) -> Value {
    let Some(n) = value.as_i64() else {
        return Value::Null;
    };
    let ms = if n < 10_000_000_000 { n * 1000 } else { n };
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| Value::from(t.format("%Y-%m-%d %H:%M UTC").to_string()))
        .unwrap_or(Value::Null)
}

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

/// A string array argument, trimmed, without blanks, at most `max`
fn str_list(args: &Value, key: &str, max: usize) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| {
            v.as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .take(max)
        .collect()
}

/// The last N minutes as the CLI's ISO 8601 start and end times
fn iso_window(minutes: u64) -> (String, String) {
    let end = chrono::Utc::now();
    let start = end - chrono::Duration::minutes(minutes as i64);
    let iso = |t: chrono::DateTime<chrono::Utc>| t.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    (iso(start), iso(end))
}

/// An ECS service's PRIMARY deployment (the one rolling out)
fn primary_deployment(service: &Value) -> Value {
    service["deployments"]
        .as_array()
        .and_then(|d| d.iter().find(|d| d["status"] == "PRIMARY"))
        .cloned()
        .unwrap_or(Value::Null)
}

impl AwsConnector {
    pub fn new(runner: Arc<dyn AwsRunner>) -> Self {
        Self { runner }
    }

    pub fn tool_defs() -> Vec<ToolDef> {
        let tool = |name: &str, description: &str, schema: Value, sensitive: bool| ToolDef {
            name: name.into(),
            description: description.into(),
            input_schema: schema,
            sensitive,
        };
        vec![
            tool(
                "alarms",
                "CloudWatch alarms in a state (default ALARM): name, reason, metric, when it changed.",
                json!({"type": "object", "properties": {
                    "state": {"type": "string", "enum": ["ALARM", "OK", "INSUFFICIENT_DATA"]},
                    "prefix": {"type": "string", "description": "alarm name prefix"},
                }}),
                false,
            ),
            tool(
                "log_groups",
                "CloudWatch log group names, optionally starting with a prefix (e.g. /aws/batch).",
                json!({"type": "object", "properties": {"prefix": {"type": "string"}}}),
                false,
            ),
            tool(
                "logs_query",
                "Run a CloudWatch Logs Insights query over the last N minutes and return its rows. Keep queries narrow (filter, stats, limit).",
                json!({"type": "object", "properties": {
                    "log_group": {"type": "string"},
                    "query": {"type": "string", "description": "Logs Insights query, e.g. fields @timestamp, @message | filter @message like /ERROR/ | sort @timestamp desc"},
                    "minutes": {"type": "integer", "minimum": 1, "maximum": MAX_MINUTES, "description": "default 60"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": MAX_ROWS, "description": "default 50"},
                }, "required": ["log_group", "query"]}),
                false,
            ),
            tool(
                "logs_tail",
                "Recent log events from a log group (or one stream), optionally matching a filter pattern.",
                json!({"type": "object", "properties": {
                    "log_group": {"type": "string"},
                    "stream": {"type": "string", "description": "one log stream, e.g. a Batch job's"},
                    "filter": {"type": "string", "description": "CloudWatch filter pattern, e.g. ERROR"},
                    "minutes": {"type": "integer", "minimum": 1, "maximum": MAX_MINUTES, "description": "default 60"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": MAX_ROWS, "description": "default 50"},
                }, "required": ["log_group"]}),
                false,
            ),
            tool(
                "batch_jobs",
                "AWS Batch jobs in a queue with a status (default FAILED): id, name, status, reason, when.",
                json!({"type": "object", "properties": {
                    "queue": {"type": "string"},
                    "status": {"type": "string", "enum": ["SUBMITTED", "PENDING", "RUNNABLE", "STARTING", "RUNNING", "SUCCEEDED", "FAILED"]},
                    "limit": {"type": "integer", "minimum": 1, "maximum": MAX_ROWS},
                }, "required": ["queue"]}),
                false,
            ),
            tool(
                "batch_describe",
                "Details of AWS Batch jobs: status, reason, exit code, definition, queue, parameters, and the log group and stream with their output.",
                json!({"type": "object", "properties": {
                    "job_ids": {"type": "array", "items": {"type": "string"}, "maxItems": 20},
                }, "required": ["job_ids"]}),
                false,
            ),
            tool(
                "metrics",
                "A CloudWatch metric over the last N minutes (e.g. AWS/ECS CPUUtilization, AWS/EC2 StatusCheckFailed): one datapoint per period, oldest first.",
                json!({"type": "object", "properties": {
                    "namespace": {"type": "string", "description": "e.g. AWS/ECS, AWS/EC2, AWS/Batch, or a custom one"},
                    "metric": {"type": "string", "description": "metric name, e.g. CPUUtilization"},
                    "dimensions": {"type": "object", "additionalProperties": {"type": "string"}, "description": "dimension name to value, e.g. ClusterName and ServiceName"},
                    "statistic": {"type": "string", "enum": ["Average", "Sum", "Minimum", "Maximum", "SampleCount"], "description": "default Average"},
                    "minutes": {"type": "integer", "minimum": 1, "maximum": MAX_MINUTES, "description": "default 60"},
                    "period": {"type": "integer", "minimum": 60, "maximum": 86400, "description": "seconds per datapoint, default 300"},
                }, "required": ["namespace", "metric"]}),
                false,
            ),
            tool(
                "ecs_services",
                "ECS services in a cluster (or the ones named): status, desired / running / pending tasks, task definition, the rollout's state and the latest events.",
                json!({"type": "object", "properties": {
                    "cluster": {"type": "string"},
                    "services": {"type": "array", "items": {"type": "string"}, "maxItems": MAX_SERVICES, "description": "default: the cluster's first 10"},
                }, "required": ["cluster"]}),
                false,
            ),
            tool(
                "ec2_status",
                "EC2 instance status: state, system and instance status checks, and scheduled events. Default: only instances with a failing check or an event.",
                json!({"type": "object", "properties": {
                    "instance_ids": {"type": "array", "items": {"type": "string"}, "maxItems": 50},
                    "all": {"type": "boolean", "description": "include healthy and stopped instances (default false)"},
                }}),
                false,
            ),
            tool(
                "ecs_redeploy",
                "Start a new deployment of an ECS service with its current task definition (replaces its tasks). Waits for the user's approval; give the reason and the evidence.",
                json!({"type": "object", "properties": {
                    "cluster": {"type": "string"},
                    "service": {"type": "string"},
                    "reason": {"type": "string", "description": "why, with the evidence"},
                }, "required": ["cluster", "service", "reason"]}),
                true,
            ),
            tool(
                "batch_restart",
                "Resubmit a failed AWS Batch job with the same definition, queue and parameters. Waits for the user's approval; give the reason and the evidence.",
                json!({"type": "object", "properties": {
                    "job_id": {"type": "string"},
                    "reason": {"type": "string", "description": "why, with the evidence"},
                }, "required": ["job_id", "reason"]}),
                true,
            ),
        ]
    }

    async fn aws(&self, args: Vec<String>) -> Result<Value, String> {
        self.runner.run(args).await
    }

    async fn alarms(&self, args: &Value) -> Result<Value, String> {
        let state = opt_str(args, "state").unwrap_or("ALARM");
        let mut call = owned(&["cloudwatch", "describe-alarms", "--state-value", state]);
        if let Some(prefix) = opt_str(args, "prefix") {
            call.extend(owned(&["--alarm-name-prefix", prefix]));
        }
        let out = self.aws(call).await?;
        let alarms: Vec<Value> = out["MetricAlarms"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| {
                json!({
                    "name": a["AlarmName"],
                    "state": a["StateValue"],
                    "reason": a["StateReason"],
                    "metric": a["MetricName"],
                    "namespace": a["Namespace"],
                    "since": a["StateUpdatedTimestamp"],
                })
            })
            .collect();
        Ok(json!({"state": state, "count": alarms.len(), "alarms": alarms}))
    }

    async fn log_groups(&self, args: &Value) -> Result<Value, String> {
        let mut call = owned(&["logs", "describe-log-groups", "--max-items", "50"]);
        if let Some(prefix) = opt_str(args, "prefix") {
            call.extend(owned(&["--log-group-name-prefix", prefix]));
        }
        let out = self.aws(call).await?;
        let groups: Vec<Value> = out["logGroups"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|g| g["logGroupName"].clone())
            .collect();
        Ok(json!({"log_groups": groups}))
    }

    async fn logs_query(&self, args: &Value) -> Result<Value, String> {
        let group = str_arg(args, "log_group")?;
        let query = str_arg(args, "query")?;
        let minutes = num(args, "minutes", 60, MAX_MINUTES);
        let limit = num(args, "limit", 50, MAX_ROWS);
        let end = now_ms() / 1000;
        let start = end - (minutes as i64) * 60;
        let started = self
            .aws(vec![
                "logs".into(),
                "start-query".into(),
                "--log-group-name".into(),
                group.into(),
                "--start-time".into(),
                start.to_string(),
                "--end-time".into(),
                end.to_string(),
                "--query-string".into(),
                query.into(),
                "--limit".into(),
                limit.to_string(),
            ])
            .await?;
        let query_id = started["queryId"]
            .as_str()
            .ok_or("CloudWatch Logs didn't start the query")?
            .to_string();
        let deadline = tokio::time::Instant::now() + QUERY_WAIT;
        loop {
            let out = self
                .aws(owned(&[
                    "logs",
                    "get-query-results",
                    "--query-id",
                    &query_id,
                ]))
                .await?;
            let status = out["status"].as_str().unwrap_or_default();
            match status {
                "Complete" => {
                    let rows: Vec<Value> = out["results"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|row| {
                            let mut map = serde_json::Map::new();
                            for cell in row.as_array().into_iter().flatten() {
                                if let (Some(field), Some(value)) =
                                    (cell["field"].as_str(), cell["value"].as_str())
                                {
                                    if field != "@ptr" {
                                        map.insert(field.into(), cut(value, MAX_MESSAGE_CHARS).into());
                                    }
                                }
                            }
                            Value::Object(map)
                        })
                        .collect();
                    return Ok(json!({
                        "log_group": group,
                        "minutes": minutes,
                        "rows": rows,
                        "statistics": out["statistics"],
                    }));
                }
                "Failed" | "Cancelled" | "Timeout" => {
                    return Err(format!("The Logs Insights query ended as {status}"))
                }
                _ if tokio::time::Instant::now() > deadline => {
                    return Err("The Logs Insights query took over a minute: narrow the time range or the filter".into())
                }
                _ => tokio::time::sleep(QUERY_POLL).await,
            }
        }
    }

    async fn logs_tail(&self, args: &Value) -> Result<Value, String> {
        let group = str_arg(args, "log_group")?;
        let minutes = num(args, "minutes", 60, MAX_MINUTES);
        let limit = num(args, "limit", 50, MAX_ROWS);
        let start = now_ms() - (minutes as i64) * 60_000;
        let mut call = vec![
            "logs".into(),
            "filter-log-events".into(),
            "--log-group-name".into(),
            group.into(),
            "--start-time".into(),
            start.to_string(),
            "--max-items".into(),
            limit.to_string(),
        ];
        if let Some(stream) = opt_str(args, "stream") {
            call.extend(owned(&["--log-stream-names", stream]));
        }
        if let Some(filter) = opt_str(args, "filter") {
            call.extend(owned(&["--filter-pattern", filter]));
        }
        let out = self.aws(call).await?;
        let events: Vec<Value> = out["events"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| {
                json!({
                    "time": when(&e["timestamp"]),
                    "stream": e["logStreamName"],
                    "message": cut(e["message"].as_str().unwrap_or_default().trim_end(), MAX_MESSAGE_CHARS),
                })
            })
            .collect();
        Ok(json!({"log_group": group, "minutes": minutes, "events": events}))
    }

    async fn batch_jobs(&self, args: &Value) -> Result<Value, String> {
        let queue = str_arg(args, "queue")?;
        let status = opt_str(args, "status").unwrap_or("FAILED");
        let limit = num(args, "limit", 25, MAX_ROWS);
        let out = self
            .aws(owned(&[
                "batch",
                "list-jobs",
                "--job-queue",
                queue,
                "--job-status",
                status,
                "--max-items",
                &limit.to_string(),
            ]))
            .await?;
        let jobs: Vec<Value> = out["jobSummaryList"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|j| {
                json!({
                    "id": j["jobId"],
                    "name": j["jobName"],
                    "status": j["status"],
                    "reason": j["statusReason"],
                    "created": when(&j["createdAt"]),
                    "stopped": when(&j["stoppedAt"]),
                })
            })
            .collect();
        Ok(json!({"queue": queue, "status": status, "jobs": jobs}))
    }

    async fn describe(&self, ids: &[String]) -> Result<Vec<Value>, String> {
        let mut call = owned(&["batch", "describe-jobs", "--jobs"]);
        call.extend(ids.iter().cloned());
        let out = self.aws(call).await?;
        Ok(out["jobs"].as_array().cloned().unwrap_or_default())
    }

    async fn batch_describe(&self, args: &Value) -> Result<Value, String> {
        let ids: Vec<String> = args
            .get("job_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| {
                v.as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
            .take(20)
            .collect();
        if ids.is_empty() {
            return Err("Give at least one job ID in 'job_ids'".into());
        }
        let jobs: Vec<Value> = self
            .describe(&ids)
            .await?
            .iter()
            .map(|j| {
                let container = &j["container"];
                let group = container["logConfiguration"]["options"]["awslogs-group"]
                    .as_str()
                    .unwrap_or("/aws/batch/job");
                json!({
                    "id": j["jobId"],
                    "name": j["jobName"],
                    "status": j["status"],
                    "reason": j["statusReason"],
                    "exit_code": container["exitCode"],
                    "container_reason": container["reason"],
                    "queue": j["jobQueue"],
                    "definition": j["jobDefinition"],
                    "parameters": j["parameters"],
                    "attempts": j["attempts"].as_array().map_or(0, Vec::len),
                    "created": when(&j["createdAt"]),
                    "stopped": when(&j["stoppedAt"]),
                    "log_group": group,
                    "log_stream": container["logStreamName"],
                })
            })
            .collect();
        Ok(json!({"jobs": jobs}))
    }

    async fn batch_restart(&self, args: &Value) -> Result<Value, String> {
        let job_id = str_arg(args, "job_id")?;
        let reason = str_arg(args, "reason")?;
        let jobs = self.describe(&[job_id.to_string()]).await?;
        let job = jobs
            .first()
            .ok_or_else(|| format!("No AWS Batch job '{job_id}'"))?;
        let status = job["status"].as_str().unwrap_or_default();
        if status != "FAILED" {
            return Err(format!(
                "Job '{job_id}' is {status}: only a FAILED job is restarted"
            ));
        }
        let field = |key: &str| {
            job[key]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("Job '{job_id}' has no {key}"))
        };
        let name = field("jobName")?;
        let queue = field("jobQueue")?;
        let definition = field("jobDefinition")?;
        let mut call = vec![
            "batch".to_string(),
            "submit-job".into(),
            "--job-name".into(),
            format!("{}-retry", name.chars().take(120).collect::<String>()),
            "--job-queue".into(),
            queue.clone(),
            "--job-definition".into(),
            definition.clone(),
        ];
        if let Some(parameters) = job["parameters"].as_object().filter(|p| !p.is_empty()) {
            call.push("--parameters".into());
            call.push(Value::Object(parameters.clone()).to_string());
        }
        let out = self.aws(call).await?;
        Ok(json!({
            "restarted": job_id,
            "new_job_id": out["jobId"],
            "new_job_name": out["jobName"],
            "queue": queue,
            "definition": definition,
            "reason": reason,
        }))
    }
}

impl AwsConnector {
    async fn metrics(&self, args: &Value) -> Result<Value, String> {
        let namespace = str_arg(args, "namespace")?;
        let metric = str_arg(args, "metric")?;
        let statistic = opt_str(args, "statistic").unwrap_or("Average");
        let minutes = num(args, "minutes", 60, MAX_MINUTES);
        let period = args
            .get("period")
            .and_then(Value::as_u64)
            .unwrap_or(300)
            .clamp(60, 86_400);
        let (start, end) = iso_window(minutes);
        let mut call = owned(&[
            "cloudwatch",
            "get-metric-statistics",
            "--namespace",
            namespace,
            "--metric-name",
            metric,
            "--start-time",
            &start,
            "--end-time",
            &end,
            "--period",
            &period.to_string(),
            "--statistics",
            statistic,
        ]);
        let dimensions: Vec<String> = args
            .get("dimensions")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(name, value)| value.as_str().map(|v| format!("Name={name},Value={v}")))
            .collect();
        if !dimensions.is_empty() {
            call.push("--dimensions".into());
            call.extend(dimensions);
        }
        let out = self.aws(call).await?;
        let mut points = out["Datapoints"].as_array().cloned().unwrap_or_default();
        // ISO timestamps sort as text
        points.sort_by(|a, b| {
            a["Timestamp"]
                .as_str()
                .unwrap_or_default()
                .cmp(b["Timestamp"].as_str().unwrap_or_default())
        });
        let points: Vec<Value> = points
            .iter()
            .map(|p| json!({"time": p["Timestamp"], "value": p[statistic], "unit": p["Unit"]}))
            .collect();
        Ok(json!({
            "namespace": namespace,
            "metric": metric,
            "statistic": statistic,
            "period_s": period,
            "minutes": minutes,
            "datapoints": points,
        }))
    }

    async fn ecs_services(&self, args: &Value) -> Result<Value, String> {
        let cluster = str_arg(args, "cluster")?;
        let mut names = str_list(args, "services", MAX_SERVICES);
        if names.is_empty() {
            let listed = self
                .aws(owned(&[
                    "ecs",
                    "list-services",
                    "--cluster",
                    cluster,
                    "--max-items",
                    &MAX_SERVICES.to_string(),
                ]))
                .await?;
            names = str_list(&listed, "serviceArns", MAX_SERVICES);
            if names.is_empty() {
                return Ok(json!({"cluster": cluster, "services": []}));
            }
        }
        let mut call = owned(&[
            "ecs",
            "describe-services",
            "--cluster",
            cluster,
            "--services",
        ]);
        call.extend(names);
        let out = self.aws(call).await?;
        let services: Vec<Value> = out["services"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| {
                let primary = primary_deployment(s);
                let events: Vec<Value> = s["events"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(SERVICE_EVENTS)
                    .map(|e| json!({"time": e["createdAt"], "message": e["message"]}))
                    .collect();
                json!({
                    "name": s["serviceName"],
                    "status": s["status"],
                    "desired": s["desiredCount"],
                    "running": s["runningCount"],
                    "pending": s["pendingCount"],
                    "task_definition": s["taskDefinition"],
                    "deployments": s["deployments"].as_array().map_or(0, Vec::len),
                    "rollout": primary["rolloutState"],
                    "rollout_reason": primary["rolloutStateReason"],
                    "events": events,
                })
            })
            .collect();
        let missing: Vec<Value> = out["failures"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| json!({"service": f["arn"], "reason": f["reason"]}))
            .collect();
        Ok(json!({"cluster": cluster, "services": services, "not_found": missing}))
    }

    async fn ec2_status(&self, args: &Value) -> Result<Value, String> {
        let ids = str_list(args, "instance_ids", 50);
        let everything =
            args.get("all").and_then(Value::as_bool).unwrap_or(false) || !ids.is_empty();
        let mut call = owned(&["ec2", "describe-instance-status"]);
        if everything {
            call.push("--include-all-instances".into());
        }
        if !ids.is_empty() {
            call.push("--instance-ids".into());
            call.extend(ids);
        }
        let out = self.aws(call).await?;
        let check = |v: &Value| v["Status"].as_str().unwrap_or("not-applicable").to_string();
        let instances: Vec<Value> = out["InstanceStatuses"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|i| {
                let events: Vec<Value> = i["Events"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|e| json!({"code": e["Code"], "description": e["Description"], "not_before": e["NotBefore"]}))
                    .collect();
                json!({
                    "id": i["InstanceId"],
                    "zone": i["AvailabilityZone"],
                    "state": i["InstanceState"]["Name"],
                    "system_check": check(&i["SystemStatus"]),
                    "instance_check": check(&i["InstanceStatus"]),
                    "events": events,
                })
            })
            // By default only what needs attention
            .filter(|i| {
                everything
                    || i["system_check"] != "ok"
                    || i["instance_check"] != "ok"
                    || i["events"].as_array().is_some_and(|e| !e.is_empty())
            })
            .collect();
        Ok(json!({"count": instances.len(), "instances": instances}))
    }

    async fn ecs_redeploy(&self, args: &Value) -> Result<Value, String> {
        let cluster = str_arg(args, "cluster")?;
        let service = str_arg(args, "service")?;
        let reason = str_arg(args, "reason")?;
        let out = self
            .aws(owned(&[
                "ecs",
                "update-service",
                "--cluster",
                cluster,
                "--service",
                service,
                "--force-new-deployment",
            ]))
            .await?;
        let s = &out["service"];
        Ok(json!({
            "redeployed": s["serviceName"],
            "cluster": cluster,
            "deployment_id": primary_deployment(s)["id"],
            "task_definition": s["taskDefinition"],
            "desired": s["desiredCount"],
            "reason": reason,
        }))
    }
}

impl Connector for AwsConnector {
    fn list_tools(&self) -> Vec<ToolDef> {
        Self::tool_defs()
    }

    fn call_tool<'a>(
        &'a self,
        name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            match name {
                "alarms" => self.alarms(args).await,
                "log_groups" => self.log_groups(args).await,
                "logs_query" => self.logs_query(args).await,
                "logs_tail" => self.logs_tail(args).await,
                "batch_jobs" => self.batch_jobs(args).await,
                "batch_describe" => self.batch_describe(args).await,
                "batch_restart" => self.batch_restart(args).await,
                "metrics" => self.metrics(args).await,
                "ecs_services" => self.ecs_services(args).await,
                "ec2_status" => self.ec2_status(args).await,
                "ecs_redeploy" => self.ecs_redeploy(args).await,
                other => Err(format!("Unknown tool '{other}'")),
            }
        })
    }

    /// The AWS CLI is installed. No AWS call: the CLI starts too slowly for
    /// a health check, and each tool call reports AWS's own errors (with the
    /// next step) anyway
    fn health_check(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            if cli_on_path() {
                Ok(())
            } else {
                Err("The AWS CLI isn't installed: install it, then set the profile in Setup".into())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Answers by the AWS service and operation, and records every call
    #[derive(Default)]
    struct Fake {
        answers: Vec<(String, Value)>,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl Fake {
        fn with(answers: &[(&str, Value)]) -> Arc<Self> {
            Arc::new(Self {
                answers: answers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect(),
                calls: Mutex::default(),
            })
        }
        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl AwsRunner for Fake {
        fn run(&self, args: Vec<String>) -> BoxFuture<'_, Result<Value, String>> {
            let key = format!("{} {}", args[0], args[1]);
            self.calls.lock().unwrap().push(args);
            let answer = self
                .answers
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
                .ok_or(format!("no fake for {key}"));
            Box::pin(async move { answer })
        }
    }

    fn failed_job() -> Value {
        json!({"jobs": [{
            "jobId": "j-1", "jobName": "etl-nightly", "status": "FAILED",
            "statusReason": "Essential container in task exited",
            "jobQueue": "arn:aws:batch:us-east-1:1:job-queue/etl", "jobDefinition": "arn:aws:batch:us-east-1:1:job-definition/etl:3",
            "parameters": {"date": "2026-10-23"},
            "attempts": [{}], "createdAt": 1792800000000_i64, "stoppedAt": 1792800600000_i64,
            "container": {"exitCode": 137, "reason": "OutOfMemoryError: Container killed", "logStreamName": "etl/default/abc"},
        }]})
    }

    #[tokio::test]
    async fn a_failed_job_links_to_its_logs() {
        let fake = Fake::with(&[("batch describe-jobs", failed_job())]);
        let aws = AwsConnector::new(fake.clone());
        let out = aws
            .call_tool("batch_describe", &json!({"job_ids": ["j-1"]}))
            .await
            .unwrap();
        let job = &out["jobs"][0];
        assert_eq!(job["exit_code"], 137);
        assert_eq!(job["log_group"], "/aws/batch/job");
        assert_eq!(job["log_stream"], "etl/default/abc");
        assert_eq!(job["stopped"], "2026-10-24 00:10 UTC");
        assert_eq!(fake.calls()[0], ["batch", "describe-jobs", "--jobs", "j-1"]);
    }

    #[tokio::test]
    async fn a_restart_resubmits_with_the_same_definition_queue_and_parameters() {
        let fake = Fake::with(&[
            ("batch describe-jobs", failed_job()),
            (
                "batch submit-job",
                json!({"jobId": "j-2", "jobName": "etl-nightly-retry"}),
            ),
        ]);
        let aws = AwsConnector::new(fake.clone());
        let out = aws
            .call_tool(
                "batch_restart",
                &json!({"job_id": "j-1", "reason": "OOM; data size normal"}),
            )
            .await
            .unwrap();
        assert_eq!(out["new_job_id"], "j-2");
        let submit = &fake.calls()[1];
        assert_eq!(
            &submit[..4],
            ["batch", "submit-job", "--job-name", "etl-nightly-retry"]
        );
        assert!(submit.contains(&"arn:aws:batch:us-east-1:1:job-definition/etl:3".to_string()));
        assert_eq!(submit.last().unwrap(), r#"{"date":"2026-10-23"}"#);
        // Only the changes need approval
        let sensitive: Vec<_> = AwsConnector::tool_defs()
            .into_iter()
            .filter(|t| t.sensitive)
            .map(|t| t.name)
            .collect();
        assert_eq!(sensitive, ["ecs_redeploy", "batch_restart"]);
    }

    #[tokio::test]
    async fn a_job_that_did_not_fail_is_not_restarted() {
        let mut job = failed_job();
        job["jobs"][0]["status"] = json!("RUNNING");
        let fake = Fake::with(&[("batch describe-jobs", job)]);
        let aws = AwsConnector::new(fake.clone());
        let error = aws
            .call_tool("batch_restart", &json!({"job_id": "j-1", "reason": "x"}))
            .await
            .unwrap_err();
        assert!(error.contains("only a FAILED job"), "{error}");
        assert_eq!(fake.calls().len(), 1, "no submit-job");
    }

    #[tokio::test]
    async fn an_insights_query_returns_rows_without_pointers() {
        let fake = Fake::with(&[
            ("logs start-query", json!({"queryId": "q-1"})),
            (
                "logs get-query-results",
                json!({"status": "Complete", "statistics": {"recordsMatched": 2.0},
                "results": [[{"field": "@timestamp", "value": "2026-10-23 05:49"}, {"field": "@message", "value": "OutOfMemoryError"}, {"field": "@ptr", "value": "x"}]]}),
            ),
        ]);
        let aws = AwsConnector::new(fake.clone());
        let out = aws
            .call_tool("logs_query", &json!({"log_group": "/aws/batch/job", "query": "filter @message like /Error/", "minutes": 120}))
            .await
            .unwrap();
        assert_eq!(out["rows"][0]["@message"], "OutOfMemoryError");
        assert!(out["rows"][0].get("@ptr").is_none());
        let start = &fake.calls()[0];
        assert_eq!(start[1], "start-query");
        let (from, to): (i64, i64) = (start[5].parse().unwrap(), start[7].parse().unwrap());
        assert_eq!(to - from, 120 * 60);
    }

    #[tokio::test]
    async fn alarms_default_to_the_ones_firing() {
        let fake = Fake::with(&[(
            "cloudwatch describe-alarms",
            json!({"MetricAlarms": [
                {"AlarmName": "etl-nightly-failed", "StateValue": "ALARM", "StateReason": "Threshold crossed", "MetricName": "FailedJobs", "Namespace": "Custom/ETL"}
            ]}),
        )]);
        let aws = AwsConnector::new(fake.clone());
        let out = aws.call_tool("alarms", &json!({})).await.unwrap();
        assert_eq!(out["count"], 1);
        assert_eq!(out["alarms"][0]["name"], "etl-nightly-failed");
        assert_eq!(
            fake.calls()[0],
            ["cloudwatch", "describe-alarms", "--state-value", "ALARM"]
        );
    }

    #[tokio::test]
    async fn metrics_come_oldest_first_with_their_dimensions() {
        let fake = Fake::with(&[(
            "cloudwatch get-metric-statistics",
            json!({"Datapoints": [
                {"Timestamp": "2026-10-27T10:05:00Z", "Average": 97.5, "Unit": "Percent"},
                {"Timestamp": "2026-10-27T10:00:00Z", "Average": 41.0, "Unit": "Percent"},
            ]}),
        )]);
        let aws = AwsConnector::new(fake.clone());
        let out = aws
            .call_tool(
                "metrics",
                &json!({"namespace": "AWS/ECS", "metric": "CPUUtilization", "dimensions": {"ClusterName": "prod"}}),
            )
            .await
            .unwrap();
        assert_eq!(out["datapoints"][0]["value"], 41.0);
        assert_eq!(out["datapoints"][1]["value"], 97.5);
        let call = &fake.calls()[0];
        assert!(call.ends_with(&[
            "--dimensions".to_string(),
            "Name=ClusterName,Value=prod".to_string()
        ]));
        assert!(call.contains(&"Average".to_string()));
    }

    #[tokio::test]
    async fn ecs_services_lists_the_cluster_and_shows_the_rollout() {
        let arn = "arn:aws:ecs:us-east-1:1:service/prod/api";
        let fake = Fake::with(&[
            ("ecs list-services", json!({"serviceArns": [arn]})),
            (
                "ecs describe-services",
                json!({"services": [{
                    "serviceName": "api", "status": "ACTIVE", "desiredCount": 2, "runningCount": 0, "pendingCount": 2,
                    "taskDefinition": "arn:aws:ecs:us-east-1:1:task-definition/api:7",
                    "deployments": [{"status": "PRIMARY", "rolloutState": "FAILED", "rolloutStateReason": "tasks failed to start"}],
                    "events": [{"createdAt": "2026-10-27T10:00:00Z", "message": "(service api) is unable to consistently start tasks successfully."}],
                }], "failures": []}),
            ),
        ]);
        let aws = AwsConnector::new(fake.clone());
        let out = aws
            .call_tool("ecs_services", &json!({"cluster": "prod"}))
            .await
            .unwrap();
        let api = &out["services"][0];
        assert_eq!(api["running"], 0);
        assert_eq!(api["rollout"], "FAILED");
        assert_eq!(api["events"].as_array().unwrap().len(), 1);
        assert_eq!(
            fake.calls()[1],
            [
                "ecs",
                "describe-services",
                "--cluster",
                "prod",
                "--services",
                arn
            ]
        );
    }

    #[tokio::test]
    async fn ec2_status_defaults_to_instances_that_need_attention() {
        let fake = Fake::with(&[(
            "ec2 describe-instance-status",
            json!({"InstanceStatuses": [
                {"InstanceId": "i-ok", "InstanceState": {"Name": "running"}, "SystemStatus": {"Status": "ok"}, "InstanceStatus": {"Status": "ok"}},
                {"InstanceId": "i-bad", "InstanceState": {"Name": "running"}, "SystemStatus": {"Status": "impaired"}, "InstanceStatus": {"Status": "ok"}},
            ]}),
        )]);
        let aws = AwsConnector::new(fake.clone());
        let out = aws.call_tool("ec2_status", &json!({})).await.unwrap();
        assert_eq!(out["count"], 1);
        assert_eq!(out["instances"][0]["id"], "i-bad");
        assert_eq!(fake.calls()[0], ["ec2", "describe-instance-status"]);
    }

    #[tokio::test]
    async fn a_redeploy_forces_a_new_deployment_of_that_service() {
        let fake = Fake::with(&[(
            "ecs update-service",
            json!({"service": {"serviceName": "api", "taskDefinition": "td:7", "desiredCount": 2,
                "deployments": [{"id": "ecs-svc/1", "status": "PRIMARY"}]}}),
        )]);
        let aws = AwsConnector::new(fake.clone());
        let out = aws
            .call_tool(
                "ecs_redeploy",
                &json!({"cluster": "prod", "service": "api", "reason": "tasks stuck after the config fix"}),
            )
            .await
            .unwrap();
        assert_eq!(out["deployment_id"], "ecs-svc/1");
        assert_eq!(
            fake.calls()[0],
            [
                "ecs",
                "update-service",
                "--cluster",
                "prod",
                "--service",
                "api",
                "--force-new-deployment"
            ]
        );
    }

    #[test]
    fn sign_in_errors_say_what_to_do() {
        assert!(
            explain_error("Error when retrieving token from sso: Token has expired")
                .contains("aws sso login")
        );
        assert!(
            explain_error("The config profile (prod) could not be found").contains("~/.aws/config")
        );
        assert!(
            explain_error("An error occurred (AccessDeniedException) when calling ListJobs")
                .contains("isn't allowed")
        );
    }
}
