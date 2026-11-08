//! What an agent may not write into the user's notes (`my-notes`): card
//! numbers and passwords. Sign-ins and payments are always handed over, so
//! a note never needs one; this catches the obvious slips (a card number
//! copied from a page, a `password:` line in a site profile).
//!
//! - A card number is a run of 13 to 19 digits, spaces or dashes allowed
//!   between them, that passes the Luhn check.
//! - A password line is `<key>: <value>` (or `=`) whose key ends in
//!   password, passcode, passwd, PIN, CVV, CVC or OTP, or is a one-time or
//!   security code, with a value. The name of a keychain secret
//!   (`Password: ACME_PASSWORD`, capitals) is fine: that's what a site
//!   profile records.

/// Why a note wasn't written, for the agent (it says what to do instead)
pub fn check(content: &str) -> Result<(), String> {
    if has_card_number(content) {
        return Err("Not written: the note has what looks like a card number. Card numbers, passwords, CVVs and one-time codes are never kept in notes: the user types them at a hand-over. Keep only the last 4 digits (\"card ending 4242\").".to_string());
    }
    if let Some(key) = content.lines().find_map(password_key) {
        return Err(format!("Not written: the note has a '{key}' line. Passwords, PINs and codes are never kept in notes. Name the keychain secret instead (\"Secret name: ACME_PASSWORD\"), or write that the user signs in at a hand-over."));
    }
    Ok(())
}

const CARD_DIGITS: std::ops::RangeInclusive<usize> = 13..=19;

/// Whether a Luhn-valid run of 13 to 19 digits is in `text`
fn has_card_number(text: &str) -> bool {
    let mut digits: Vec<u32> = Vec::new();
    // A separator is allowed only between two digits
    let mut pending_separator = false;
    let mut found = false;
    let mut end_run = |digits: &mut Vec<u32>| {
        found |= CARD_DIGITS.contains(&digits.len()) && luhn(digits);
        digits.clear();
    };
    for c in text.chars() {
        match c {
            '0'..='9' => {
                digits.push(c.to_digit(10).unwrap_or_default());
                pending_separator = false;
            }
            ' ' | '-' if !digits.is_empty() && !pending_separator => pending_separator = true,
            _ => {
                end_run(&mut digits);
                pending_separator = false;
            }
        }
    }
    end_run(&mut digits);
    found
}

fn luhn(digits: &[u32]) -> bool {
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| match (!i.is_multiple_of(2), d * 2) {
            (true, doubled) if doubled > 9 => doubled - 9,
            (true, doubled) => doubled,
            (false, _) => d,
        })
        .sum();
    sum.is_multiple_of(10)
}

const SECRET_WORDS: &[&str] = &["password", "passcode", "passwd", "pin", "cvv", "cvc", "otp"];
const SECRET_KEYS: &[&str] = &["one-time code", "one time code", "security code"];

/// The key of a `password: value` line, if `line` is one
fn password_key(line: &str) -> Option<String> {
    let plain: String = line.chars().filter(|c| !matches!(c, '*' | '`')).collect();
    let plain = plain
        .trim_start()
        .trim_start_matches(['-', '+', '>', ' ', '\t']);
    let at = plain.find([':', '='])?;
    let key = plain[..at].trim().to_ascii_lowercase();
    let value = plain[at + 1..].trim();
    let last_word = key.rsplit([' ', '-', '_']).next().unwrap_or_default();
    let secret_key = SECRET_WORDS.contains(&last_word) || SECRET_KEYS.contains(&key.as_str());
    (secret_key && !value.is_empty() && !is_secret_name(value)).then_some(key)
}

/// A keychain secret's name, as the browser types it (`SITE_PASSWORD`)
fn is_secret_name(value: &str) -> bool {
    value.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && value
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_numbers_are_refused_with_or_without_spaces() {
        for card in [
            "Card: 4242424242424242",
            "Paid with 4111 1111 1111 1111 today",
            "amex 3782-822463-10005",
            "5555555555554444.",
        ] {
            let refused = check(card).unwrap_err();
            assert!(refused.contains("card number"), "{card}: {refused}");
        }
    }

    #[test]
    fn other_numbers_are_kept() {
        for fine in [
            "Customer number 55-1234",
            "Card ending 4242",
            "Phone +1 617 555 0100",
            // 16 digits that fail the Luhn check (an order number)
            "Order 4242424242424241",
            // Too long for a card
            "Reference 42424242424242424242424",
            "Paid $84.00 on 2026-11-08, confirmation 7781-22",
        ] {
            assert_eq!(check(fine), Ok(()), "{fine}");
        }
    }

    #[test]
    fn password_lines_are_refused() {
        for line in [
            "password: hunter2",
            "- **Password:** hunter2",
            "Site password = hunter2",
            "PIN: 1234",
            "CVV: 123",
            "One-time code: 552211",
        ] {
            let refused = check(&format!("# acme.com\n\n{line}\n")).unwrap_err();
            assert!(refused.contains("Not written"), "{line}: {refused}");
        }
    }

    #[test]
    fn a_site_profile_without_secrets_is_written() {
        let profile = "# acme-energy.com

- Account: electricity, customer number 55-1234
- Sign in: hand-over (the user has a passkey)
- Secret name: ACME_PASSWORD
- Password: ACME_PASSWORD
- Password:
- Never keep passwords here.
- Billing: Account › Billing › Pay bill
";
        assert_eq!(check(profile), Ok(()));
    }
}
