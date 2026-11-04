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
}
