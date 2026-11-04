//! The user's AWS profiles, as the AWS SDK parses them
//! (`aws_config::profile::load`: `~/.aws/config` + `~/.aws/credentials`,
//! `AWS_CONFIG_FILE` / `AWS_SHARED_CREDENTIALS_FILE`, `sso-session`
//! sections). Nandi only maps the SDK's profiles to what Settings shows; it
//! parses no AWS file itself and never reads key values out.

use aws_config::profile::{Profile, ProfileSet};
use aws_types::os_shim_internal::{Env, Fs};

use super::aws_auth::{AwsProfile, AwsProfileKind};

/// Every profile the SDK finds, the selected one (`AWS_PROFILE`, else
/// `default`) marked
pub async fn load_profiles(fs: &Fs, env: &Env) -> Result<Vec<AwsProfile>, String> {
    let set = aws_config::profile::load(fs, env, &Default::default(), None)
        .await
        .map_err(|e| format!("Could not read your AWS config: {e}"))?;
    Ok(profiles(&set))
}

pub fn profiles(set: &ProfileSet) -> Vec<AwsProfile> {
    let selected = set.selected_profile().to_string();
    let mut names: Vec<&str> = set.profiles().collect();
    names.sort_unstable();
    names
        .into_iter()
        .filter_map(|name| set.get_profile(name).map(|p| profile(set, p, &selected)))
        .collect()
}

fn profile(set: &ProfileSet, profile: &Profile, selected: &str) -> AwsProfile {
    let get = |key: &str| profile.get(key).map(str::to_string);
    let sso_session = get("sso_session");
    // `sso_region` / `sso_start_url` live on the sso-session section when
    // the profile uses one
    let session = sso_session.as_deref().and_then(|s| set.sso_session(s));
    let from_session =
        |key: &str| get(key).or_else(|| session.and_then(|s| s.get(key)).map(str::to_string));
    let has_static_keys = profile.get("aws_access_key_id").is_some()
        && profile.get("aws_secret_access_key").is_some();
    let kind = if sso_session.is_some() || profile.get("sso_start_url").is_some() {
        AwsProfileKind::Sso
    } else if profile.get("login_session").is_some() {
        AwsProfileKind::ConsoleLogin
    } else if profile.get("role_arn").is_some() {
        AwsProfileKind::AssumeRole
    } else if profile.get("credential_process").is_some() {
        AwsProfileKind::CredentialProcess
    } else if has_static_keys {
        AwsProfileKind::StaticKeys
    } else {
        AwsProfileKind::Other
    };
    AwsProfile {
        name: profile.name().to_string(),
        kind,
        region: get("region"),
        sso_session,
        sso_start_url: from_session("sso_start_url"),
        sso_region: from_session("sso_region"),
        sso_account_id: get("sso_account_id"),
        sso_role_name: get("sso_role_name"),
        role_arn: get("role_arn"),
        source_profile: get("source_profile"),
        has_static_keys,
        is_default: profile.name() == selected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
[default]
region = us-east-1

[profile work]
sso_session = corp
sso_account_id = 111122223333
sso_role_name = BedrockUser
region = us-west-2

[profile legacy]
sso_start_url = https://legacy.awsapps.com/start
sso_region = eu-west-1
sso_account_id = 444455556666
sso_role_name = Dev

[profile admin]
role_arn = arn:aws:iam::111122223333:role/Admin
source_profile = default

[profile vault]
credential_process = aws-vault export --format=json work

[profile console]
login_session = arn:aws:iam::111122223333:user/Ada
region = us-east-2

[sso-session corp]
sso_start_url = https://corp.awsapps.com/start
sso_region = us-east-1
sso_registration_scopes = sso:account:access
"#;

    const CREDENTIALS: &str =
        "[default]\naws_access_key_id = AKIAEXAMPLE\naws_secret_access_key = super-secret-value\n";

    async fn fixture(selected: Option<&str>) -> Vec<AwsProfile> {
        // Explicit file paths: the SDK rewrites `~/` paths with the OS's
        // separators, which a fake filesystem can't match on Windows
        let fs = Fs::from_slice(&[
            ("/fixture/config", CONFIG),
            ("/fixture/credentials", CREDENTIALS),
        ]);
        let mut vars = vec![
            ("HOME", "/home/ada"),
            ("AWS_CONFIG_FILE", "/fixture/config"),
            ("AWS_SHARED_CREDENTIALS_FILE", "/fixture/credentials"),
        ];
        if let Some(profile) = selected {
            vars.push(("AWS_PROFILE", profile));
        }
        load_profiles(&fs, &Env::from_slice(&vars)).await.unwrap()
    }

    #[tokio::test]
    async fn sso_sessions_and_profile_kinds_come_from_the_sdk() {
        let profiles = fixture(None).await;
        let by = |name: &str| profiles.iter().find(|p| p.name == name).unwrap();
        assert_eq!(
            profiles.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["admin", "console", "default", "legacy", "vault", "work"]
        );
        let work = by("work");
        assert_eq!(work.kind, AwsProfileKind::Sso);
        assert_eq!(work.sso_session.as_deref(), Some("corp"));
        assert_eq!(
            work.sso_start_url.as_deref(),
            Some("https://corp.awsapps.com/start"),
            "inherited from the sso-session"
        );
        assert_eq!(work.sso_region.as_deref(), Some("us-east-1"));
        assert_eq!(work.region.as_deref(), Some("us-west-2"));
        assert_eq!(by("legacy").kind, AwsProfileKind::Sso);
        assert_eq!(by("legacy").sso_region.as_deref(), Some("eu-west-1"));
        assert_eq!(by("admin").kind, AwsProfileKind::AssumeRole);
        assert_eq!(by("admin").source_profile.as_deref(), Some("default"));
        assert_eq!(by("vault").kind, AwsProfileKind::CredentialProcess);
        assert_eq!(by("console").kind, AwsProfileKind::ConsoleLogin);
        let default = by("default");
        assert_eq!(default.kind, AwsProfileKind::StaticKeys);
        assert!(default.has_static_keys);
        assert!(default.is_default);
    }

    #[tokio::test]
    async fn aws_profile_selects_the_default_and_keys_never_surface() {
        let profiles = fixture(Some("work")).await;
        let defaults: Vec<_> = profiles.iter().filter(|p| p.is_default).collect();
        assert_eq!(defaults.len(), 1);
        assert_eq!(defaults[0].name, "work");
        let json = serde_json::to_string(&profiles).unwrap();
        assert!(!json.contains("super-secret-value"));
        assert!(!json.contains("AKIAEXAMPLE"));
    }

    #[tokio::test]
    async fn no_aws_config_is_no_profiles() {
        let profiles = load_profiles(
            &Fs::from_slice(&[]),
            &Env::from_slice(&[("HOME", "/nobody")]),
        )
        .await
        .unwrap();
        assert!(profiles.is_empty());
    }
}
