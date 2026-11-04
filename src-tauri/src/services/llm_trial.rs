//! The free trial: Nandi's own DeepSeek key, for the first
//! [`TRIAL_TOKENS`] tokens on this computer.
//!
//! The key is baked in at build time (`NANDI_TRIAL_DEEPSEEK_KEY`, a CI
//! secret; never in the repo), masked so it isn't plain text in the binary
//! (`build.rs`). A build without it has no trial. Once the tokens are used
//! up the key is never unmasked or sent again; nothing else on the
//! computer holds it, so there is nothing to delete. Masking only stops a
//! casual look: anyone who takes the binary apart can recover the key, so
//! the DeepSeek account behind it needs a spending cap. It runs only
//! [`TRIAL_MODEL`], and only on the host: like every key it never enters a
//! container.
//!
//! The trial has three [`TrialMode`]s. It starts in `Auto`: calls use it
//! only when the user has nothing of their own (`llm_keys::usable_model`
//! tries their keys and sign-ins first). The first time a call on a model
//! of their own works (a saved key, a Claude Code sign-in, Ollama, a model
//! from `llm.yaml`) it turns `Off` for good. A sign-in that's only found,
//! not proven, doesn't count: a stale login would leave them with nothing. The user can turn it back `On` in
//! Settings › Models & runtimes while tokens are left; then the default model runs on
//! it again until they're used up or the user saves another key.
//!
//! The mode and the tokens used live in the keychain next to the keys, so
//! clearing `~/.nandi` doesn't reset them. When the keychain can't be
//! read, the trial counts as used up and off: a failed read never hands
//! the allowance back.
//!
//! Each call reserves its tokens before it's sent ([`reserve`]): the
//! prompt's estimated size plus the answer's cap, which must fit in what's
//! left after the calls already running. The reservation is settled with
//! the tokens the provider reports; a call that fails is charged its
//! prompt, and one that's cancelled midway its whole reservation, since
//! DeepSeek bills for those too.

use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use specta::Type;

use crate::llm::{LlmRequest, Usage};
use crate::services::secrets::Secrets;

/// Input + output tokens a computer gets on the trial key
pub const TRIAL_TOKENS: u32 = 100_000;
/// The one model the trial runs (the cheapest DeepSeek model)
pub const TRIAL_MODEL: &str = "deepseek:deepseek-flash";
/// Keychain entry holding the tokens used so far, as a decimal number
const USED_ENTRY: &str = "nandi.trial.deepseek.used";
/// Keychain entry holding the default model from before the user turned
/// the trial on, so turning it off gives it back
const PREVIOUS_ENTRY: &str = "nandi.trial.deepseek.previous-model";

pub const TRIAL_USED_UP: &str = "Your 100,000 free tokens are used up. Add your own key (Anthropic, OpenAI, or DeepSeek) in Settings › Models & runtimes, sign in to Claude Code (`claude`), or run a local model with Ollama.";
/// What to do instead of the trial, for errors that aren't "used up"
const TRIAL_OWN_MODEL: &str = "Add your own key (Anthropic, OpenAI, or DeepSeek) in Settings › Models & runtimes, sign in to Claude Code (`claude`), or run a local model with Ollama.";
const TRIAL_BUSY: &str =
    "The free tokens left are in use by other calls right now. Try again when they finish.";

/// Tokens reserved by calls in flight. The lock also serializes the
/// counter's read-modify-write across concurrent calls.
static IN_FLIGHT: Mutex<u32> = Mutex::new(0);

fn in_flight() -> std::sync::MutexGuard<'static, u32> {
    IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner())
}

/// The trial's progress on this computer
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TrialStatus {
    pub used: u32,
    pub limit: u32,
    /// Not turned off (by the user, or by their own model)
    pub enabled: bool,
    /// The default model runs on the trial right now
    pub active: bool,
}

// `TRIAL_KEY_MASKED` and `TRIAL_KEY_PAD`, written by `build.rs`
include!(concat!(env!("OUT_DIR"), "/trial_key.rs"));

/// The trial key this build ships with, unmasked once in memory. Tests
/// never see one, so a developer's build environment can't change their
/// outcome.
fn trial_key() -> Option<&'static str> {
    static KEY: OnceLock<Option<String>> = OnceLock::new();
    if cfg!(test) {
        return None;
    }
    KEY.get_or_init(|| unmask(TRIAL_KEY_MASKED, TRIAL_KEY_PAD))
        .as_deref()
}

fn unmask(masked: &[u8], pad: &[u8]) -> Option<String> {
    let key: Vec<u8> = masked.iter().zip(pad).map(|(m, p)| m ^ p).collect();
    String::from_utf8(key).ok().filter(|key| !key.is_empty())
}

/// Keychain entry holding the [`TrialMode`] (`on` / `off`; none is `Auto`)
const MODE_ENTRY: &str = "nandi.trial.deepseek.mode";

/// Whether model calls may use the trial
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialMode {
    /// Only while the user has no model of their own (the start)
    Auto,
    /// Never: the user has their own model, or turned it off
    Off,
    /// The user turned it back on: the default model runs on it
    On,
}

/// Whether this build ships a trial key (without unmasking it)
fn shipped() -> bool {
    !cfg!(test) && !TRIAL_KEY_MASKED.is_empty()
}

/// Tokens used so far; `None` when the counter can't be trusted (the
/// keychain can't be read, or the entry isn't a number), which callers
/// treat as used up
fn used(secrets: &Secrets) -> Option<u32> {
    match secrets.read(USED_ENTRY) {
        Ok(None) => Some(0),
        Ok(Some(value)) => value.trim().parse().ok(),
        Err(e) => {
            log::warn!("Free trial: {e}");
            None
        }
    }
}

fn left(secrets: &Secrets) -> u32 {
    used(secrets).map_or(0, |used| TRIAL_TOKENS.saturating_sub(used))
}

fn mode(secrets: &Secrets) -> TrialMode {
    match secrets.read(MODE_ENTRY) {
        Ok(None) => TrialMode::Auto,
        Ok(Some(value)) => match value.as_str() {
            "on" => TrialMode::On,
            "off" => TrialMode::Off,
            _ => TrialMode::Auto,
        },
        Err(e) => {
            log::warn!("Free trial: {e}");
            TrialMode::Off
        }
    }
}

fn set_mode(secrets: &Secrets, mode: TrialMode) -> Result<(), String> {
    match mode {
        TrialMode::Auto => secrets.delete(MODE_ENTRY),
        TrialMode::Off => secrets.set(MODE_ENTRY, "off"),
        TrialMode::On => secrets.set(MODE_ENTRY, "on"),
    }
}

/// Calls may use the trial (not off, tokens left)
pub fn available(secrets: &Secrets) -> bool {
    available_with(shipped(), secrets)
}

fn available_with(shipped: bool, secrets: &Secrets) -> bool {
    shipped && mode(secrets) != TrialMode::Off && left(secrets) > 0
}

/// The user turned the trial back on and tokens are left: `deepseek:`
/// calls run on it before any key
pub fn chosen(secrets: &Secrets) -> bool {
    chosen_with(shipped(), secrets)
}

fn chosen_with(shipped: bool, secrets: &Secrets) -> bool {
    shipped && mode(secrets) == TrialMode::On && left(secrets) > 0
}

/// Whether this build has a trial that is used up
pub fn used_up(secrets: &Secrets) -> bool {
    shipped() && left(secrets) == 0
}

/// A call on the user's own model just worked: an `Auto` trial turns off
/// for good (a trial the user turned `On` stays on). Checked once per run
/// of the app, so calls don't each read the keychain.
pub fn own_model_worked(secrets: &Secrets) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static DONE: AtomicBool = AtomicBool::new(false);
    if !DONE.swap(true, Ordering::Relaxed) {
        own_model_found_with(shipped(), secrets);
    }
}

fn own_model_found_with(shipped: bool, secrets: &Secrets) {
    if shipped && mode(secrets) == TrialMode::Auto {
        match set_mode(secrets, TrialMode::Off) {
            Ok(()) => log::info!("The user has their own model; the free trial is off"),
            Err(e) => log::warn!("Could not turn off the free trial: {e}"),
        }
    }
}

/// The user just set up a model of their own (saved a key, chose Ollama):
/// the trial turns off, even one they had turned back on
pub fn own_model_set_up(secrets: &Secrets) {
    own_model_set_up_with(shipped(), secrets);
}

fn own_model_set_up_with(shipped: bool, secrets: &Secrets) {
    if shipped && mode(secrets) != TrialMode::Off {
        match set_mode(secrets, TrialMode::Off) {
            Ok(()) => log::info!("The user set up their own model; the free trial is off"),
            Err(e) => log::warn!("Could not turn off the free trial: {e}"),
        }
    }
}

/// Turns the trial on (only while tokens are left) or off
pub fn set_enabled(secrets: &Secrets, enabled: bool) -> Result<(), String> {
    set_enabled_with(shipped(), secrets, enabled)
}

fn set_enabled_with(shipped: bool, secrets: &Secrets, enabled: bool) -> Result<(), String> {
    if !shipped {
        return Err("This build of Nandi has no free trial.".to_string());
    }
    if !enabled {
        return set_mode(secrets, TrialMode::Off);
    }
    if left(secrets) == 0 {
        return Err(TRIAL_USED_UP.to_string());
    }
    set_mode(secrets, TrialMode::On)
}

/// The trial's progress, `None` when this build has no trial. `active`
/// says whether calls use it now (from `llm_keys`).
pub fn status(secrets: &Secrets, active: bool) -> Option<TrialStatus> {
    shipped().then(|| TrialStatus {
        used: used(secrets).unwrap_or(TRIAL_TOKENS).min(TRIAL_TOKENS),
        limit: TRIAL_TOKENS,
        enabled: mode(secrets) != TrialMode::Off,
        active,
    })
}

/// The prompt's size in tokens, estimated on the high side (a token is
/// about four characters; this counts three, and the tool schemas too)
pub fn estimate_input(request: &LlmRequest) -> u32 {
    let bytes = serde_json::to_vec(request).map_or(0, |json| json.len());
    u32::try_from(bytes / 3).unwrap_or(u32::MAX)
}

/// One trial call's reservation. Settle it with the provider's usage; if
/// it's dropped unsettled (the call was cancelled midway), the whole
/// reservation is charged.
#[must_use = "settle the call with its usage, or it is charged in full"]
pub struct TrialCall {
    secrets: Secrets,
    input: u32,
    reserved: u32,
    settled: bool,
}

impl TrialCall {
    /// The call finished: charge what the provider reported
    pub fn settle(mut self, usage: &Usage) {
        self.settled = true;
        let tokens = usage.input_tokens.saturating_add(usage.output_tokens);
        release(&self.secrets, self.reserved, tokens);
    }

    /// The call failed: charge its prompt, which may have been billed
    pub fn failed(mut self) {
        self.settled = true;
        release(&self.secrets, self.reserved, self.input);
    }
}

impl Drop for TrialCall {
    fn drop(&mut self) {
        if !self.settled {
            release(&self.secrets, self.reserved, self.reserved);
        }
    }
}

/// The key, the output-token cap and the reservation for one trial call
/// whose prompt is about `input` tokens, or why there's none. An off or
/// used-up trial fails before the key is unmasked.
pub fn reserve(
    secrets: &Secrets,
    input: u32,
    max_tokens: u32,
) -> Result<(&'static str, u32, TrialCall), String> {
    reserve_with(shipped(), trial_key, secrets, input, max_tokens)
}

fn reserve_with(
    shipped: bool,
    key: impl FnOnce() -> Option<&'static str>,
    secrets: &Secrets,
    input: u32,
    max_tokens: u32,
) -> Result<(&'static str, u32, TrialCall), String> {
    if !available_with(shipped, secrets) {
        return Err(if shipped && left(secrets) == 0 {
            TRIAL_USED_UP.to_string()
        } else {
            "DeepSeek API key not configured. Add it in Settings › Models & runtimes.".to_string()
        });
    }
    let key = key().ok_or_else(|| TRIAL_USED_UP.to_string())?;
    let mut reserved = in_flight();
    let free = left(secrets).saturating_sub(*reserved);
    // The prompt has to fit, with room left for an answer
    if free <= input {
        return Err(if *reserved > 0 {
            TRIAL_BUSY.to_string()
        } else {
            format!(
                "Only {free} free tokens are left, not enough for this request. {TRIAL_OWN_MODEL}"
            )
        });
    }
    let max_tokens = max_tokens.min(free - input);
    let call = TrialCall {
        secrets: secrets.clone(),
        input,
        reserved: input + max_tokens,
        settled: false,
    };
    *reserved += call.reserved;
    Ok((key, max_tokens, call))
}

/// Ends a reservation of `reserved` tokens, charging `tokens` of them
fn release(secrets: &Secrets, reserved: u32, tokens: u32) {
    let mut in_flight = in_flight();
    *in_flight = in_flight.saturating_sub(reserved);
    charge(secrets, tokens);
}

/// Adds tokens to the counter. A counter that can't be read isn't
/// rewritten: that would replace the real total with this call's.
fn charge(secrets: &Secrets, tokens: u32) {
    let Some(used) = used(secrets) else {
        log::warn!("Free trial: not counting {tokens} tokens, the counter can't be read");
        return;
    };
    let total = used.saturating_add(tokens);
    if let Err(e) = secrets.set(USED_ENTRY, &total.to_string()) {
        log::warn!("Could not save the free-trial token count: {e}");
    }
    if total >= TRIAL_TOKENS {
        log::info!("The free trial is used up ({total} tokens)");
    }
}

/// Keeps the default model from before the trial was turned on
pub fn remember_default(secrets: &Secrets, model: &str) {
    if model == TRIAL_MODEL {
        return;
    }
    if let Err(e) = secrets.set(PREVIOUS_ENTRY, model) {
        log::warn!("Could not remember the default model: {e}");
    }
}

/// The default model from before the trial was turned on, once
pub fn take_remembered_default(secrets: &Secrets) -> Option<String> {
    let model = secrets.read(PREVIOUS_ENTRY).ok().flatten()?;
    let _ = secrets.delete(PREVIOUS_ENTRY);
    Some(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u32, output: u32) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            cache_hit_tokens: 0,
        }
    }

    #[test]
    fn a_build_without_a_trial_key_has_no_trial() {
        let secrets = Secrets::in_memory();
        assert!(!available(&secrets));
        assert!(!chosen(&secrets));
        assert!(!used_up(&secrets));
        assert_eq!(status(&secrets, false), None);
        assert!(set_enabled(&secrets, true).is_err());
        assert!(reserve(&secrets, 100, 4096)
            .err()
            .unwrap()
            .contains("DeepSeek API key not configured"));
        // Nothing is written to the keychain
        own_model_set_up(&secrets);
        own_model_found_with(false, &secrets);
        assert_eq!(secrets.get(MODE_ENTRY).unwrap(), None);
    }

    #[test]
    fn calls_count_down_the_allowance() {
        let secrets = Secrets::in_memory();
        assert!(available_with(true, &secrets));

        charge(&secrets, 61_000);
        assert_eq!(left(&secrets), 39_000);
        charge(&secrets, 39_500);
        // Overshooting the last call is fine; nothing is left
        assert_eq!(left(&secrets), 0);
        assert_eq!(used(&secrets), Some(100_500));
        assert!(!available_with(true, &secrets));
        // Used up: it can't be turned back on
        assert!(set_enabled_with(true, &secrets, true)
            .unwrap_err()
            .contains("used up"));
    }

    #[test]
    fn an_own_model_turns_the_trial_off_until_the_user_turns_it_on() {
        let secrets = Secrets::in_memory();
        assert_eq!(mode(&secrets), TrialMode::Auto);
        assert!(!chosen_with(true, &secrets));

        // A key or sign-in found: off for good, even once it's gone
        own_model_found_with(true, &secrets);
        assert_eq!(mode(&secrets), TrialMode::Off);
        assert!(!available_with(true, &secrets));

        // The user turns it back on; finding their model again keeps it on
        set_enabled_with(true, &secrets, true).unwrap();
        assert!(chosen_with(true, &secrets));
        own_model_found_with(true, &secrets);
        assert_eq!(mode(&secrets), TrialMode::On);

        // Saving another key turns it off again
        own_model_set_up_with(true, &secrets);
        assert_eq!(mode(&secrets), TrialMode::Off);
        assert!(!chosen_with(true, &secrets));

        // And the user can turn it off themselves
        set_enabled_with(true, &secrets, true).unwrap();
        set_enabled_with(true, &secrets, false).unwrap();
        assert_eq!(mode(&secrets), TrialMode::Off);
    }

    #[test]
    fn the_key_is_unmasked_with_its_pad() {
        let key = b"sk-trial";
        let pad = [7u8, 200, 13, 99, 1, 0, 255, 42];
        let masked: Vec<u8> = key.iter().zip(pad).map(|(k, p)| k ^ p).collect();
        assert_ne!(masked, key);
        assert_eq!(unmask(&masked, &pad).as_deref(), Some("sk-trial"));
        assert_eq!(unmask(&[], &[]), None);
    }

    #[test]
    fn an_unreadable_counter_counts_as_used_up_and_is_not_rewritten() {
        let secrets = Secrets::in_memory();
        secrets.make_unreadable(USED_ENTRY);
        assert_eq!(used(&secrets), None);
        assert_eq!(left(&secrets), 0);
        assert!(!available_with(true, &secrets));
        // A call's tokens don't replace the real total
        charge(&secrets, 2_000);
        assert!(secrets.read(USED_ENTRY).is_err());

        // A value that isn't a number can't reset the allowance either
        secrets.set(USED_ENTRY, "not a number").unwrap();
        assert_eq!(left(&secrets), 0);
    }

    #[test]
    fn an_unreadable_mode_counts_as_off() {
        let secrets = Secrets::in_memory();
        secrets.make_unreadable(MODE_ENTRY);
        assert_eq!(mode(&secrets), TrialMode::Off);
        assert!(!available_with(true, &secrets));
    }

    /// Reservations share one in-flight total; these tests take turns
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: Mutex<()> = Mutex::new(());
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn key() -> Option<&'static str> {
        Some("sk-test")
    }

    #[test]
    fn a_call_reserves_its_tokens_and_settles_what_it_used() {
        let _serial = serial();
        let secrets = Secrets::in_memory();
        charge(&secrets, 90_000);

        // 10k left: the answer is capped to what the prompt leaves
        let (_, max_tokens, call) = reserve_with(true, key, &secrets, 4_000, 8_192).unwrap();
        assert_eq!(max_tokens, 6_000);
        // A parallel call can't spend the same tokens
        let busy = reserve_with(true, key, &secrets, 1_000, 8_192)
            .err()
            .unwrap();
        assert!(busy.contains("in use by other calls"));

        call.settle(&usage(4_100, 900));
        assert_eq!(used(&secrets), Some(95_000));
        assert_eq!(*in_flight(), 0);
    }

    #[test]
    fn a_prompt_that_does_not_fit_is_refused() {
        let _serial = serial();
        let secrets = Secrets::in_memory();
        charge(&secrets, 99_000);
        let refused = reserve_with(true, key, &secrets, 5_000, 8_192)
            .err()
            .unwrap();
        assert!(refused.contains("Only 1000 free tokens are left"));
        assert_eq!(used(&secrets), Some(99_000));
    }

    #[test]
    fn failed_and_cancelled_calls_are_charged() {
        let _serial = serial();
        let secrets = Secrets::in_memory();

        // A failure is charged its prompt
        let (_, _, call) = reserve_with(true, key, &secrets, 3_000, 1_000).unwrap();
        call.failed();
        assert_eq!(used(&secrets), Some(3_000));

        // A cancelled call (dropped midway) is charged its whole reservation
        let (_, _, call) = reserve_with(true, key, &secrets, 2_000, 1_000).unwrap();
        drop(call);
        assert_eq!(used(&secrets), Some(6_000));
        assert_eq!(*in_flight(), 0);
    }

    #[test]
    fn turning_the_trial_off_gives_back_the_old_default() {
        let secrets = Secrets::in_memory();
        remember_default(&secrets, "claude:claude-sonnet-5");
        assert_eq!(
            take_remembered_default(&secrets).as_deref(),
            Some("claude:claude-sonnet-5")
        );
        // Only once, and the trial's own model is never remembered
        assert_eq!(take_remembered_default(&secrets), None);
        remember_default(&secrets, TRIAL_MODEL);
        assert_eq!(take_remembered_default(&secrets), None);
    }

    #[test]
    fn the_prompt_estimate_counts_high() {
        let request = LlmRequest {
            model: TRIAL_MODEL.to_string(),
            system: Some("x".repeat(3_000)),
            messages: vec![],
            tools: vec![],
            max_tokens: None,
        };
        // 3,000 characters is about 750 real tokens; the estimate is over that
        assert!(estimate_input(&request) >= 1_000);
    }
}
