//! The OS's own unlock prompt, for a Locked key.
//!
//! | Platform | Prompt                                                       |
//! | -------- | ------------------------------------------------------------ |
//! | Windows  | Windows Hello (face, fingerprint or PIN), `UserConsentVerifier` |
//! | macOS    | Not yet: a Locked key asks each time instead                 |
//! | Linux    | Not yet: a Locked key asks each time instead                 |
//!
//! Where there is no prompt, or it can't be shown, Nandi never invents a
//! weaker unlock: the Lock box falls back to an approval card (Ask each
//! time) and says so. A prompt the person cancels or fails releases
//! nothing, and never falls back.

use async_trait::async_trait;

/// What the person did with the prompt
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnlockOutcome {
    /// The OS confirmed it is them
    Verified,
    /// They cancelled, or the OS couldn't confirm them
    Declined,
    /// No prompt could be shown
    Unavailable,
}

/// The OS's unlock prompt (a fake in tests)
#[async_trait]
pub trait SystemUnlock: Send + Sync {
    /// The prompt's name for people (`Windows Hello`), or `None` when this
    /// computer has none set up
    async fn available(&self) -> Option<&'static str>;

    /// Shows the prompt with `message` and waits for the person
    async fn verify(&self, message: &str) -> UnlockOutcome;
}

/// This computer's own prompt
pub struct OsUnlock;

#[async_trait]
impl SystemUnlock for OsUnlock {
    async fn available(&self) -> Option<&'static str> {
        #[cfg(windows)]
        {
            let ready = tokio::task::spawn_blocking(windows_hello::available)
                .await
                .unwrap_or(false);
            ready.then_some(windows_hello::NAME)
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    async fn verify(&self, message: &str) -> UnlockOutcome {
        #[cfg(windows)]
        {
            let message = message.to_string();
            tokio::task::spawn_blocking(move || windows_hello::verify(&message))
                .await
                .unwrap_or(UnlockOutcome::Unavailable)
        }
        #[cfg(not(windows))]
        {
            let _ = message;
            UnlockOutcome::Unavailable
        }
    }
}

#[cfg(windows)]
mod windows_hello {
    use windows::core::{factory, HSTRING};
    use windows::Security::Credentials::UI::{
        UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
    };
    use windows::Win32::System::WinRT::IUserConsentVerifierInterop;
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    use windows_future::IAsyncOperation;

    use super::UnlockOutcome;

    pub const NAME: &str = "Windows Hello";

    /// Whether Windows Hello is set up for this person on this computer
    pub fn available() -> bool {
        UserConsentVerifier::CheckAvailabilityAsync()
            .and_then(|op| op.get())
            .is_ok_and(|a| a == UserConsentVerifierAvailability::Available)
    }

    /// The prompt, over the window in front (Nandi's, when the person is
    /// using it), so it isn't hidden behind it
    pub fn verify(message: &str) -> UnlockOutcome {
        let message = HSTRING::from(message);
        let asked: windows::core::Result<IAsyncOperation<UserConsentVerificationResult>> =
            // SAFETY: a plain Win32 call with no arguments
            match unsafe { GetForegroundWindow() } {
                window if !window.is_invalid() => {
                    factory::<UserConsentVerifier, IUserConsentVerifierInterop>().and_then(
                        // SAFETY: `window` is a live top-level window handle
                        |interop| unsafe {
                            interop.RequestVerificationForWindowAsync(window, &message)
                        },
                    )
                }
                _ => UserConsentVerifier::RequestVerificationAsync(&message),
            };
        match asked.and_then(|op| op.get()) {
            Ok(UserConsentVerificationResult::Verified) => UnlockOutcome::Verified,
            Ok(
                UserConsentVerificationResult::DeviceNotPresent
                | UserConsentVerificationResult::NotConfiguredForUser
                | UserConsentVerificationResult::DisabledByPolicy
                | UserConsentVerificationResult::DeviceBusy,
            ) => UnlockOutcome::Unavailable,
            Ok(_) => UnlockOutcome::Declined,
            Err(e) => {
                log::warn!("Windows Hello couldn't ask: {e}");
                UnlockOutcome::Unavailable
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether this computer has a prompt, without showing it.
    /// Run with: cargo test system_unlock_here -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "reads this computer's unlock setup"]
    async fn system_unlock_here() {
        println!("System unlock: {:?}", OsUnlock.available().await);
        #[cfg(windows)]
        println!(
            "Windows Hello: {:?}",
            windows::Security::Credentials::UI::UserConsentVerifier::CheckAvailabilityAsync()
                .and_then(|op| op.get())
        );
    }

    /// The real prompt on this computer: someone must answer it.
    /// Run with: cargo test real_system_unlock -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "shows this computer's unlock prompt"]
    async fn real_system_unlock_prompt() {
        let unlock = OsUnlock;
        let name = unlock.available().await;
        println!("System unlock: {name:?}");
        if name.is_some() {
            let outcome = unlock
                .verify("Nandi test: unlock the key 'test' for 1 minute")
                .await;
            println!("Outcome: {outcome:?}");
        }
    }
}
