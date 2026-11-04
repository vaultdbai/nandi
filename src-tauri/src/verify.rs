//! `verify --agent <id>`: checks what an end-to-end run of the agent needs
//! (skill.md, Docker, the runtime image, the state database, the model's API
//! key) and prints a report. Makes no LLM calls and starts no container.

use rusqlite::{Connection, OpenFlags};

use crate::db::SCHEMA_VERSION;
use crate::llm::{self, Provider};
use crate::runtime::{self, DEFAULT_RUNTIME_IMAGE};
use crate::services::agents::{self, SkillConfig};
use crate::services::llm_keys;
use crate::services::secrets::Secrets;
use crate::services::APP_IDENTIFIER;

#[derive(Default)]
struct Report {
    failures: u32,
}

impl Report {
    fn pass(&mut self, message: impl AsRef<str>) {
        println!("✅ {}", message.as_ref());
    }

    fn fail(&mut self, message: impl AsRef<str>) {
        println!("❌ {}", message.as_ref());
        self.failures += 1;
    }

    fn note(&mut self, message: impl AsRef<str>) {
        println!("⚠️  {}", message.as_ref());
    }
}

/// Runs every check and returns the process exit code (1 if any failed).
pub fn run(agent_id: &str) -> i32 {
    println!("Nandi verify: agent '{agent_id}'\n");
    let mut report = Report::default();

    let skill = match agents::load_skill_from_config(agent_id) {
        Ok(skill) => {
            report.pass(format!(
                "skill.md parses ({})",
                skill.dir.join("skill.md").display()
            ));
            Some(skill)
        }
        Err(e) => {
            report.fail(e);
            None
        }
    };
    if let Some(skill) = &skill {
        check_tools(&mut report, skill);
    }

    let image = skill
        .as_ref()
        .and_then(|skill| skill.image.clone())
        .unwrap_or_else(|| DEFAULT_RUNTIME_IMAGE.to_string());
    check_docker(&mut report, &image);
    check_db(&mut report);
    if let Some(skill) = &skill {
        check_api_key(&mut report, skill);
    }

    println!();
    if report.failures == 0 {
        println!("All checks passed.");
        0
    } else {
        println!("{} check(s) failed.", report.failures);
        1
    }
}

fn check_tools(report: &mut Report, skill: &SkillConfig) {
    report.pass(format!("Model: {}", skill.model));
    if skill.allows_tool("shell") {
        report.pass("The shell tool is allowed");
    } else {
        report.fail("The shell tool is not in allowed-tools; the e2e test needs it");
    }
    if skill.needs_approval("shell") {
        report.note("shell requires approval: approve the call in the UI when asked");
    }
}

fn check_docker(report: &mut Report, image: &str) {
    let checked = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
        .and_then(|rt| rt.block_on(runtime::image_present(image)));
    match checked {
        Ok(present) => {
            report.pass("Docker is running");
            if present {
                report.pass(format!("Image {image} exists"));
            } else if image == DEFAULT_RUNTIME_IMAGE {
                report.fail(format!(
                    "Image {image} not found, or from an older Nandi. Build it with: docker build -t {image} runtimes/nandi-runtime"
                ));
            } else {
                report.fail(format!(
                    "Image {image} not found locally, or from an older Nandi"
                ));
            }
        }
        Err(e) => {
            report.fail(e);
            report.note(format!("Image {image} not checked (Docker unavailable)"));
        }
    }
}

fn check_db(report: &mut Report) {
    let path = match crate::utils::nandi_config::state_db_path() {
        Ok(path) => path,
        Err(e) => return report.fail(e),
    };
    if !path.exists() {
        return report.fail(format!(
            "{} not found. Launch Nandi once to create it",
            path.display()
        ));
    }
    let version = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .and_then(|conn| conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0)));
    match version {
        Ok(SCHEMA_VERSION) => {
            report.pass(format!("{} has schema v{SCHEMA_VERSION}", path.display()));
        }
        Ok(version) => report.fail(format!(
            "{} has schema v{version}, but this build of Nandi uses v{SCHEMA_VERSION}",
            path.display()
        )),
        Err(e) => report.fail(format!("Failed to read {}: {e}", path.display())),
    }
}

/// Checks that a key is stored. The key itself is never printed.
fn check_api_key(report: &mut Report, skill: &SkillConfig) {
    let Ok((provider, _)) = llm::parse_model(&skill.model) else {
        return; // load_skill already rejected it
    };
    if provider == Provider::Ollama {
        return report.pass("Ollama needs no API key");
    }
    match llm_keys::require_api_key(&Secrets::keychain(APP_IDENTIFIER), provider) {
        Ok(_) => report.pass(format!(
            "API key for {} is stored in the system keychain",
            skill.model
        )),
        Err(e) => report.fail(e),
    }
}
