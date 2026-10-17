//! Validation of what people type when enrolling and managing their account.

use crate::text::clean_line;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalid {
    UsernameLength,
    UsernameCharacters,
    PasswordLength,
    PasswordContainsUsername,
    PasswordTooSimple,
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UsernameLength => "Usernames are 3 to 32 characters.",
            Self::UsernameCharacters => {
                "Usernames use letters a–z, digits, dots, dashes and underscores, and start with a letter or digit."
            }
            Self::PasswordLength => "Passwords are 12 to 1024 characters.",
            Self::PasswordContainsUsername => "Passwords can't contain the username.",
            Self::PasswordTooSimple => "That password is too simple.",
        })
    }
}

/// Normalises and checks a username: ASCII lower case, `[a-z0-9._-]{3,32}`, starting with a
/// letter or digit. Look-alike characters can't sneak in because only ASCII is accepted.
pub fn username(raw: &str) -> Result<String, Invalid> {
    let u = raw.trim().to_ascii_lowercase();
    if !(3..=32).contains(&u.len()) {
        return Err(Invalid::UsernameLength);
    }
    let first_ok = u.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit());
    if !first_ok || !u.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-'))
    {
        return Err(Invalid::UsernameCharacters);
    }
    Ok(u)
}

/// Checks a new password (the breached-password list is checked separately by the server).
pub fn password(pw: &str, username: &str) -> Result<(), Invalid> {
    let n = pw.chars().count();
    if !(12..=1024).contains(&n) {
        return Err(Invalid::PasswordLength);
    }
    if !username.is_empty() && pw.to_lowercase().contains(&username.to_lowercase()) {
        return Err(Invalid::PasswordContainsUsername);
    }
    let distinct: std::collections::HashSet<char> = pw.chars().collect();
    if distinct.len() < 4 {
        return Err(Invalid::PasswordTooSimple);
    }
    Ok(())
}

/// Cleans a device or passkey name: at most 64 characters, no control or bidi characters;
/// falls back to `fallback` when nothing is left.
pub fn label(raw: &str, fallback: &str) -> String {
    let c = clean_line(raw, 64);
    if c.is_empty() { fallback.to_owned() } else { c }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames() {
        assert_eq!(username("  Ada.Example "), Ok("ada.example".into()));
        assert_eq!(username("ab"), Err(Invalid::UsernameLength));
        assert_eq!(username(&"a".repeat(33)), Err(Invalid::UsernameLength));
        for bad in [".ada", "-ada", "ad a", "adа", "ada/..", "ada@x", "ada\u{200B}x"] {
            assert!(username(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn passwords() {
        assert!(password("correct horse battery", "ada").is_ok());
        assert_eq!(password("short", "ada"), Err(Invalid::PasswordLength));
        assert_eq!(password(&"x".repeat(1025), "ada"), Err(Invalid::PasswordLength));
        assert_eq!(password("my name is ADA really", "ada"), Err(Invalid::PasswordContainsUsername));
        assert_eq!(password("aaaaaaaaaaaaaaa", "zed"), Err(Invalid::PasswordTooSimple));
        assert_eq!(password("abababababababab", "zed"), Err(Invalid::PasswordTooSimple));
        // Length counts characters, not bytes.
        assert!(password("ééééééabcdef", "zed").is_ok());
    }

    #[test]
    fn labels() {
        assert_eq!(label("  My\u{202E}Phone ", "Device"), "MyPhone");
        assert_eq!(label("\u{0000}\u{200B}", "Device"), "Device");
        assert_eq!(label(&"p".repeat(100), "Device").len(), 64);
    }
}
