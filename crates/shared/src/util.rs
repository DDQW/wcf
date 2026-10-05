//! Small helpers shared by server and browser (validation, formatting).

use uuid::Uuid;

pub const USERNAME_MIN: usize = 3;
pub const USERNAME_MAX: usize = 32;
pub const PASSWORD_MIN: usize = 8;

/// Usernames: 3–32 chars, ASCII letters, digits, `_`, `-`, `.`; must start
/// with a letter or digit. They appear in URLs (`/u/name`) and @mentions.
pub fn validate_username(name: &str) -> Result<(), &'static str> {
    let len = name.chars().count();
    if len < USERNAME_MIN {
        return Err("Der Benutzername muss mindestens 3 Zeichen lang sein.");
    }
    if len > USERNAME_MAX {
        return Err("Der Benutzername darf höchstens 32 Zeichen lang sein.");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err("Erlaubt sind nur Buchstaben (a–z), Ziffern sowie _ - .");
    }
    if !name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) {
        return Err("Der Benutzername muss mit einem Buchstaben oder einer Ziffer beginnen.");
    }
    Ok(())
}

pub fn validate_password(pw: &str) -> Result<(), &'static str> {
    if pw.chars().count() < PASSWORD_MIN {
        return Err("Das Passwort muss mindestens 8 Zeichen lang sein.");
    }
    if pw.len() > 512 {
        return Err("Das Passwort ist zu lang.");
    }
    Ok(())
}

/// Deliberately loose: one `@`, something on both sides, a dot in the domain.
pub fn validate_email(email: &str) -> Result<(), &'static str> {
    let email = email.trim();
    let Some((local, domain)) = email.split_once('@') else {
        return Err("Bitte gib eine gültige E-Mail-Adresse ein.");
    };
    if local.is_empty()
        || domain.len() < 3
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || email.contains(char::is_whitespace)
        || email.len() > 254
    {
        return Err("Bitte gib eine gültige E-Mail-Adresse ein.");
    }
    Ok(())
}

/// Up to two initials for avatar placeholders.
pub fn initials(display_name: &str) -> String {
    let mut parts = display_name
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_' || c == '.')
        .filter(|p| !p.is_empty());
    let first = parts.next().and_then(|p| p.chars().next());
    let second = parts.next_back().and_then(|p| p.chars().next());
    let mut out = String::new();
    if let Some(c) = first {
        out.extend(c.to_uppercase());
    }
    if let Some(c) = second {
        out.extend(c.to_uppercase());
    }
    if out.is_empty() {
        out.push('?');
    }
    out
}

/// Stable hue (0–359) derived from an id, for placeholder avatar colours.
pub fn hue_for(id: &Uuid) -> u16 {
    let bytes = id.as_bytes();
    let mut h: u32 = 2166136261;
    for b in bytes {
        h ^= *b as u32;
        h = h.wrapping_mul(16777619);
    }
    (h % 360) as u16
}

/// "1,5 MB" style sizes (German decimal comma).
pub fn format_bytes(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let s = if value >= 100.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    };
    format!("{} {}", s.replace('.', ","), UNITS[unit])
}

/// Truncates to `max` characters (not bytes), appending "…" if cut.
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out = out.trim_end().to_string();
    out.push('…');
    out
}

/// Estimated reading time in minutes (≈200 words/minute, at least 1).
pub fn reading_time_minutes(text: &str) -> u32 {
    let words = text.split_whitespace().count() as u32;
    (words / 200).max(1)
}

/// German singular/plural helper: `plural(1, "Antwort", "Antworten")`.
pub fn plural<'a>(n: i64, one: &'a str, many: &'a str) -> String {
    format!("{} {}", n, if n == 1 { one } else { many })
}

/// "1:05:09" / "4:07" style durations.
pub fn format_duration(secs: f64) -> String {
    let total = secs.max(0.0).round() as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// URL slug from arbitrary text (German umlauts transliterated).
pub fn slug_base(input: &str) -> String {
    let mut out = String::new();
    for c in input.to_lowercase().chars() {
        match c {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            c if c.is_ascii_alphanumeric() => out.push(c),
            _ => out.push('-'),
        }
    }
    let joined = out
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    joined.chars().take(80).collect::<String>().trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames() {
        assert!(validate_username("dominik").is_ok());
        assert!(validate_username("a.b-c_d").is_ok());
        assert!(validate_username("ab").is_err());
        assert!(validate_username("_abc").is_err());
        assert!(validate_username("mit leer").is_err());
        assert!(validate_username("ümlaut").is_err());
    }

    #[test]
    fn emails() {
        assert!(validate_email("a@b.de").is_ok());
        assert!(validate_email("ab.de").is_err());
        assert!(validate_email("a@b").is_err());
        assert!(validate_email("a b@c.de").is_err());
    }

    #[test]
    fn initials_work() {
        assert_eq!(initials("Dominik Wölki"), "DW");
        assert_eq!(initials("anna"), "A");
        assert_eq!(initials(""), "?");
    }

    #[test]
    fn bytes_format() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1,5 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024), "5,0 GB");
    }

    #[test]
    fn truncation() {
        assert_eq!(truncate_chars("Hallo Welt", 20), "Hallo Welt");
        assert_eq!(truncate_chars("Hallo Welt", 6), "Hallo…");
    }

    #[test]
    fn slugs() {
        assert_eq!(slug_base("Über die Größe!"), "ueber-die-groesse");
        assert_eq!(slug_base("  --  "), "");
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration(65.0), "1:05");
        assert_eq!(format_duration(3909.0), "1:05:09");
    }
}
