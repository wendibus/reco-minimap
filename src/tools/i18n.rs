//! Language of the program's own messages (German, English, French, Spanish).
//!
//! Every message is written in all four languages where it is printed, with [`tr!`](crate::tr)
//! (a format string per language, same arguments) or [`trs!`](crate::trs) (plain text). The
//! language is one process-wide setting: `--lang`, else `RECO_MINIMAP_LANG`, else the system
//! locale, else English. The interface sets it per request to the language chosen on the page.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    De,
    En,
    Fr,
    Es,
}

static LANG: AtomicU8 = AtomicU8::new(1);

impl Lang {
    /// `de`, `de_DE.UTF-8`, `fr-CA` ... by the first two letters.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().get(..2)?.to_ascii_lowercase().as_str() {
            "de" => Some(Lang::De),
            "en" => Some(Lang::En),
            "fr" => Some(Lang::Fr),
            "es" => Some(Lang::Es),
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::De => "de",
            Lang::En => "en",
            Lang::Fr => "fr",
            Lang::Es => "es",
        }
    }
}

/// The language messages are currently written in.
pub fn lang() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        0 => Lang::De,
        2 => Lang::Fr,
        3 => Lang::Es,
        _ => Lang::En,
    }
}

pub fn set_lang(l: Lang) {
    LANG.store(l as u8, Ordering::Relaxed);
}

/// Language from the environment: `RECO_MINIMAP_LANG`, then the locale variables; English otherwise.
pub fn from_env() -> Lang {
    ["RECO_MINIMAP_LANG", "LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|k| std::env::var(k).ok()).find_map(|v| Lang::parse(&v)).unwrap_or(Lang::En)
}

/// A message with arguments: one format string per language (German, English, French, Spanish),
/// then the arguments as for `format!`. All four must take the same arguments.
#[macro_export]
macro_rules! tr {
    ($de:literal, $en:literal, $fr:literal, $es:literal $(, $($arg:tt)*)?) => {
        match $crate::tools::i18n::lang() {
            $crate::tools::i18n::Lang::De => format!($de $(, $($arg)*)?),
            $crate::tools::i18n::Lang::En => format!($en $(, $($arg)*)?),
            $crate::tools::i18n::Lang::Fr => format!($fr $(, $($arg)*)?),
            $crate::tools::i18n::Lang::Es => format!($es $(, $($arg)*)?),
        }
    };
}

/// A fixed text in the current language (German, English, French, Spanish) as `&'static str`.
#[macro_export]
macro_rules! trs {
    ($de:literal, $en:literal, $fr:literal, $es:literal) => {
        match $crate::tools::i18n::lang() {
            $crate::tools::i18n::Lang::De => $de,
            $crate::tools::i18n::Lang::En => $en,
            $crate::tools::i18n::Lang::Fr => $fr,
            $crate::tools::i18n::Lang::Es => $es,
        }
    };
}
