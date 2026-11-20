//! `NANDI_*` environment variables. An empty value counts as unset.

use std::ffi::OsString;

/// `name` if set and not empty
pub fn var_os(name: &str) -> Option<OsString> {
    lookup(name, |key| std::env::var_os(key))
}

/// [`var_os`] as a string (`None` when it isn't valid Unicode)
pub fn var(name: &str) -> Option<String> {
    var_os(name).and_then(|value| value.into_string().ok())
}

/// A setting that turns a feature on: `run_time` (its env var) when set,
/// else `build_time` (the same name read with `option_env!` when the app
/// was built). Trimmed, no trailing slash; blank counts as unset. `None`:
/// the feature isn't in this build.
pub fn feature_url(run_time: Option<String>, build_time: Option<&str>) -> Option<String> {
    run_time
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .or_else(|| build_time.map(str::trim).filter(|url| !url.is_empty()))
        .map(|url| url.trim_end_matches('/').to_string())
}

/// `name` through `get`, if set and not empty
fn lookup<T: AsRef<std::ffi::OsStr>>(name: &str, get: impl Fn(&str) -> Option<T>) -> Option<T> {
    get(name).filter(|value| !value.as_ref().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn an_empty_value_counts_as_unset() {
        assert_eq!(
            lookup("NANDI_HOME", env(&[("NANDI_HOME", "/a")])).as_deref(),
            Some("/a")
        );
        assert_eq!(lookup("NANDI_HOME", env(&[("NANDI_HOME", "")])), None);
        assert_eq!(lookup("NANDI_HOME", env(&[])), None::<String>);
    }

    #[test]
    fn a_feature_url_comes_from_run_time_then_build_time() {
        assert_eq!(feature_url(None, None), None);
        assert_eq!(feature_url(Some("  ".into()), Some("")), None);
        assert_eq!(
            feature_url(None, Some("https://built.example/")).as_deref(),
            Some("https://built.example")
        );
        assert_eq!(
            feature_url(
                Some(" http://127.0.0.1:8790/ ".into()),
                Some("https://built.example")
            )
            .as_deref(),
            Some("http://127.0.0.1:8790")
        );
    }
}
