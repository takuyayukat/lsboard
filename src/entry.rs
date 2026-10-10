//! entry: board 直下の空ファイル。ファイル名が正本。
//!
//! `{status}__{sid}__{agent}__{name}[ {message}]`

use crate::sid::Sid;
use crate::status::Status;

pub const MAX_FILENAME_BYTES: usize = 255;
pub const MAX_MESSAGE_CHARS: usize = 30;
/// name の上限。message を空にしてもファイル名が 255 バイトに収まる長さ。
pub const MAX_NAME_CHARS: usize = 64;
const SEP: &str = "__";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub status: Status,
    pub sid: Sid,
    pub agent: String,
    pub name: String,
    pub message: String,
}

/// `[a-z0-9-]+` か。agent と name に使う。
pub fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// 任意の文字列を token に寄せる。空になれば None。
pub fn to_token(s: &str) -> Option<String> {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let t = out.trim_matches('-').to_string();
    if t.is_empty() { None } else { Some(t) }
}

/// token を MAX_NAME_CHARS に切り詰める（末尾の `-` は除く）。空になれば `x`。
pub fn clip_name(token: &str) -> String {
    let t: String = token.chars().take(MAX_NAME_CHARS).collect();
    let t = t.trim_end_matches('-').to_string();
    if t.is_empty() { "x".to_string() } else { t }
}

impl Entry {
    pub fn new(status: Status, sid: Sid, agent: &str, name: &str) -> Entry {
        Entry {
            status,
            sid,
            agent: agent.to_string(),
            name: name.to_string(),
            message: String::new(),
        }
    }

    fn compose(&self, message: &str) -> String {
        let mut s = format!(
            "{}{SEP}{}{SEP}{}{SEP}{}",
            self.status, self.sid, self.agent, self.name
        );
        if !message.is_empty() {
            s.push(' ');
            s.push_str(message);
        }
        s
    }

    /// ファイル名。255 バイトに収まるまで message を削る。
    pub fn filename(&self) -> String {
        let mut msg = self.message.clone();
        loop {
            let name = self.compose(&msg);
            if name.len() <= MAX_FILENAME_BYTES || msg.is_empty() {
                return name;
            }
            msg.pop();
            msg = collapse(&msg);
        }
    }

    pub fn parse(filename: &str) -> Option<Entry> {
        let parts: Vec<&str> = filename.splitn(4, SEP).collect();
        if parts.len() != 4 {
            return None;
        }
        let status = parts[0].parse().ok()?;
        let sid = Sid::parse(parts[1])?;
        let agent = parts[2];
        if !is_token(agent) {
            return None;
        }
        let (name, message) = match parts[3].split_once(' ') {
            Some((n, m)) => (n, m),
            None => (parts[3], ""),
        };
        if !is_token(name) {
            return None;
        }
        Some(Entry {
            status,
            sid,
            agent: agent.to_string(),
            name: name.to_string(),
            message: message.to_string(),
        })
    }
}

/// Unicode の一般カテゴリ Cf（書式文字）。std にないので範囲を列挙する。
fn is_format(c: char) -> bool {
    matches!(
        u32::from(c),
        0x00AD
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

/// 空白を 1 つにまとめ、前後の空白と末尾の `.` を除く。
fn collapse(s: &str) -> String {
    let joined = s
        .split(' ')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    joined.trim_end_matches('.').trim().to_string()
}

/// message を人が見て安全な形に整える（concept §4）。
pub fn sanitize_message(s: &str) -> String {
    let replaced: String = s
        .chars()
        .map(|c| {
            if c.is_control() || is_format(c) || c.is_whitespace() || "/\\<>:\"|?*_".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let collapsed = collapse(&replaced);
    let truncated: String = collapsed.chars().take(MAX_MESSAGE_CHARS).collect();
    collapse(&truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(s: &str) -> Sid {
        Sid::parse(s).unwrap()
    }

    #[test]
    fn filename_round_trip() {
        let mut e = Entry::new(Status::Waiting, sid("k3f9.01"), "claude", "explorer");
        e.message = "approve Bash".into();
        let n = e.filename();
        assert_eq!(n, "10-waiting__k3f9.01__claude__explorer approve Bash");
        assert_eq!(Entry::parse(&n).unwrap(), e);

        let e2 = Entry::new(Status::Idle, sid("m7q2"), "claude", "worker");
        assert_eq!(e2.filename(), "40-idle__m7q2__claude__worker");
        assert_eq!(Entry::parse(&e2.filename()).unwrap(), e2);
    }

    #[test]
    fn parse_rejects_bad_names() {
        assert!(Entry::parse("").is_none());
        assert!(Entry::parse("40-idle__m7q2__claude").is_none());
        assert!(Entry::parse("41-idle__m7q2__claude__worker").is_none());
        assert!(Entry::parse("40-idle__M7Q2__claude__worker").is_none());
        assert!(Entry::parse("40-idle__m7q2__Claude__worker").is_none());
        assert!(Entry::parse("40-idle__m7q2__claude__wor_ker").is_none());
        assert!(Entry::parse(".lsboard").is_none());
    }

    #[test]
    fn message_sanitizing_vectors() {
        assert_eq!(sanitize_message("approve Bash"), "approve Bash");
        assert_eq!(
            sanitize_message("承認待ち: design r4\n"),
            "承認待ち design r4"
        );
        assert_eq!(
            sanitize_message("run tests/unit\tthen gate..."),
            "run tests unit then gate"
        );
        assert_eq!(sanitize_message("snake_case__name"), "snake case name");
        assert_eq!(sanitize_message("pick a\u{202E}redaeh"), "pick a redaeh");
        assert_eq!(
            sanitize_message("この文字列は三十文字を超えるのでコードポイント単位で切り詰められる"),
            "この文字列は三十文字を超えるのでコードポイント単位で切り詰め"
        );
        assert_eq!(
            sanitize_message("abcdefghijklmnopqrstuvwxyz012 ...tail"),
            "abcdefghijklmnopqrstuvwxyz012"
        );
        assert_eq!(sanitize_message("   "), "");
    }

    #[test]
    fn filename_fits_255_bytes() {
        let mut e = Entry::new(Status::Waiting, sid("k3f9"), "claude", "worker");
        e.message = "字".repeat(MAX_MESSAGE_CHARS); // 90 bytes, fits
        assert!(e.filename().len() <= MAX_FILENAME_BYTES);
        e.name = "n".repeat(200);
        let n = e.filename();
        assert!(n.len() <= MAX_FILENAME_BYTES);
        let parsed = Entry::parse(&n).unwrap();
        assert_eq!(parsed.name, e.name);
        assert!(parsed.message.chars().count() < MAX_MESSAGE_CHARS);
    }

    #[test]
    fn tokens() {
        assert!(is_token("general-purpose"));
        assert!(!is_token("General"));
        assert!(!is_token(""));
        assert_eq!(to_token("General Purpose!"), Some("general-purpose".into()));
        assert_eq!(to_token("___"), None);
        let long = format!("{}-{}", "a".repeat(63), "b".repeat(20));
        assert_eq!(clip_name(&long), "a".repeat(63));
        assert_eq!(clip_name("abc"), "abc");
        assert_eq!(clip_name(&"-".repeat(70)), "x");
    }
}
