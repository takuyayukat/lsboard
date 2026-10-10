//! status: 人の注意が要る順に番号が付いた 5 つの状態。

use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Status {
    Waiting,
    Working,
    Background,
    Idle,
    Ended,
}

impl Status {
    pub const ALL: [Status; 5] = [
        Status::Waiting,
        Status::Working,
        Status::Background,
        Status::Idle,
        Status::Ended,
    ];

    pub fn number(self) -> u8 {
        match self {
            Status::Waiting => 10,
            Status::Working => 20,
            Status::Background => 30,
            Status::Idle => 40,
            Status::Ended => 90,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Waiting => "waiting",
            Status::Working => "working",
            Status::Background => "background",
            Status::Idle => "idle",
            Status::Ended => "ended",
        }
    }

    /// `wait` / `watch` の既定の対象: 人の注意が要るか、手が空いたか、終わったか。
    pub const ATTENTION: [Status; 3] = [Status::Waiting, Status::Idle, Status::Ended];
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}-{}", self.number(), self.label())
    }
}

impl FromStr for Status {
    type Err = String;

    /// `10-waiting` でも `waiting` でも受け付ける。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (num, label) = match s.split_once('-') {
            Some((n, l)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => (Some(n), l),
            _ => (None, s),
        };
        Status::ALL
            .into_iter()
            .find(|st| {
                st.label() == label && num.is_none_or(|n| n == format!("{:02}", st.number()))
            })
            .ok_or_else(|| format!("unknown status: {s}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_parse_round_trip() {
        for st in Status::ALL {
            let s = st.to_string();
            assert_eq!(s.parse::<Status>().unwrap(), st);
            assert_eq!(st.label().parse::<Status>().unwrap(), st);
        }
        assert_eq!("10-waiting".parse::<Status>().unwrap(), Status::Waiting);
        assert!("11-waiting".parse::<Status>().is_err());
        assert!("foo".parse::<Status>().is_err());
    }

    #[test]
    fn orders_by_attention() {
        assert!(Status::Waiting < Status::Working);
        assert!(Status::Working < Status::Background);
        assert!(Status::Background < Status::Idle);
        assert!(Status::Idle < Status::Ended);
        let mut v = vec![Status::Ended, Status::Idle, Status::Waiting];
        v.sort();
        assert_eq!(v, [Status::Waiting, Status::Idle, Status::Ended]);
    }
}
