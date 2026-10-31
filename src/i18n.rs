//! Language of the messages meant for the user: admin API errors, Touch ID
//! reasons and vault errors follow the macOS language (French, or English
//! otherwise). Messages meant for the agent, on the proxy port, are always in
//! English.

use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
}

static LANG: OnceLock<Lang> = OnceLock::new();

/// `TOKEN_PROXY_LANG` (`fr`, `en`) wins over the first macOS preferred language.
pub fn lang() -> Lang {
    *LANG.get_or_init(|| {
        let code = std::env::var("TOKEN_PROXY_LANG").unwrap_or_else(|_| system_language());
        if code.to_ascii_lowercase().starts_with("fr") { Lang::Fr } else { Lang::En }
    })
}

fn system_language() -> String {
    objc2_foundation::NSLocale::preferredLanguages()
        .firstObject()
        .map(|code| code.to_string())
        .unwrap_or_default()
}

/// `tr!("français {x}", "English {x}")`: the message in the user's language.
macro_rules! tr {
    ($fr:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        match $crate::i18n::lang() {
            $crate::i18n::Lang::Fr => format!($fr $(, $arg)*),
            $crate::i18n::Lang::En => format!($en $(, $arg)*),
        }
    };
}
