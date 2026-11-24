//! A paired device's PIN: chosen on the desktop when the person confirms
//! the phone, typed on the phone with every approve or decline, and
//! checked here. It is kept only as an Argon2id hash (a PHC string in
//! `own_devices.pin_hash`), never in the clear and never on the relay.
//!
//! [`MAX_TRIES`] wrong PINs in a row lock the device until the person
//! unlocks it on the desktop.

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};

/// The fewest digits a PIN has
pub const MIN_DIGITS: usize = 6;
/// The most digits a PIN has
pub const MAX_DIGITS: usize = 12;
/// Wrong PINs in a row that lock the device
pub const MAX_TRIES: u32 = 5;

/// Argon2id with OWASP's minimum (19 MiB, 2 passes, 1 lane). Tests use
/// the smallest cost so they stay fast; the algorithm is the same.
fn hasher() -> Argon2<'static> {
    #[cfg(not(test))]
    let params = Params::new(19 * 1024, 2, 1, None);
    #[cfg(test)]
    let params = Params::new(Params::MIN_M_COST, 1, 1, None);
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        params.expect("the PIN hash's parameters are valid"),
    )
}

/// Why `pin` can't be a device's PIN, or `None` when it can: 6 to 12
/// digits, not all the same digit, not a straight run (`123456`)
pub fn problem(pin: &str) -> Option<String> {
    let digits: Vec<u8> = pin.bytes().collect();
    if !digits.iter().all(u8::is_ascii_digit) {
        return Some("A PIN is digits only".to_string());
    }
    if !(MIN_DIGITS..=MAX_DIGITS).contains(&digits.len()) {
        return Some(format!("A PIN has {MIN_DIGITS} to {MAX_DIGITS} digits"));
    }
    let steps: Vec<i16> = digits
        .windows(2)
        .map(|pair| i16::from(pair[1]) - i16::from(pair[0]))
        .collect();
    if steps.iter().all(|step| *step == 0) {
        return Some("A PIN can't be one digit repeated".to_string());
    }
    if steps.iter().all(|step| *step == 1) || steps.iter().all(|step| *step == -1) {
        return Some("A PIN can't be a straight run like 123456".to_string());
    }
    None
}

/// The PHC string of `pin`, with a fresh salt
pub fn hash(pin: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    hasher()
        .hash_password(pin.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| format!("Couldn't keep the PIN: {e}"))
}

/// Whether `pin` is the one `stored` (a PHC string) was made from. A
/// stored hash that doesn't read never matches.
pub fn matches(pin: &str, stored: &str) -> bool {
    PasswordHash::new(stored)
        .is_ok_and(|parsed| hasher().verify_password(pin.as_bytes(), &parsed).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pin_is_six_to_twelve_digits_and_not_a_trivial_one() {
        for good in ["482916", "000001", "975318642013"] {
            assert_eq!(problem(good), None, "{good}");
        }
        for bad in [
            "",
            "12345",
            "1234567890123",
            "48291a",
            "482 916",
            "111111",
            "123456",
            "987654",
        ] {
            assert!(problem(bad).is_some(), "{bad}");
        }
    }

    #[test]
    fn only_the_slow_hash_is_kept_and_it_checks_the_pin() {
        let stored = hash("482916").unwrap();
        assert!(stored.starts_with("$argon2id$"), "{stored}");
        assert!(!stored.contains("482916"));
        assert!(matches("482916", &stored));
        assert!(!matches("482917", &stored));
        // Salted: the same PIN never hashes the same twice
        assert_ne!(hash("482916").unwrap(), stored);
        assert!(!matches("482916", "not a hash"));
    }
}
