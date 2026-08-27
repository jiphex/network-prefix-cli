//! Terminal colour, hand-rolled.
//!
//! A colour crate would be a third dependency for what amounts to a handful of
//! escape sequences, so this does it directly. Only the basic eight colours are
//! used, which every terminal supports and which respect the user's own theme
//! rather than imposing particular RGB values on them.
//!
//! The rules colour has to follow to stay out of the way:
//!
//! - never when the output is not a terminal, so pipes and files stay clean
//! - never for `--json` or `--quiet`, which exist to be parsed
//! - never when `NO_COLOR` is set (<https://no-color.org>) or `TERM=dumb`
//! - always when explicitly asked, so `--color=always | less -R` works

use std::io::IsTerminal;

#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ValueEnum, Default)]
pub enum When {
    /// Colour only when writing to a terminal.
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct Style {
    on: bool,
}

impl Style {
    /// Decide once, up front, so every call site stays a simple lookup.
    pub fn new(when: When) -> Style {
        Style {
            on: match when {
                When::Always => true,
                When::Never => false,
                When::Auto => std::io::stdout().is_terminal() && !suppressed_by_environment(),
            },
        }
    }

    /// For messages on stderr, which is a different stream and may be a
    /// terminal when stdout is not.
    pub fn for_stderr(when: When) -> Style {
        Style {
            on: match when {
                When::Always => true,
                When::Never => false,
                When::Auto => std::io::stderr().is_terminal() && !suppressed_by_environment(),
            },
        }
    }

    pub fn plain() -> Style {
        Style { on: false }
    }

    fn wrap(&self, code: &str, text: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    /// Section headings.
    pub fn bold(&self, text: &str) -> String {
        self.wrap("1", text)
    }

    /// The prefix under inspection, in the report's opening line. One combined
    /// sequence rather than nesting bold around prefix, which would emit two
    /// resets and cancel the bold early for anything appended after it.
    pub fn title(&self, text: &str) -> String {
        self.wrap("1;36", text)
    }

    /// Field labels and asides, which should recede.
    pub fn dim(&self, text: &str) -> String {
        self.wrap("2", text)
    }

    /// Prefixes and addresses - the thing the reader is actually looking for.
    pub fn prefix(&self, text: &str) -> String {
        self.wrap("36", text)
    }

    /// Something worked: an allocation was granted.
    pub fn good(&self, text: &str) -> String {
        self.wrap("32", text)
    }

    /// Something to look at twice: documentation space, reserved ranges.
    pub fn warn(&self, text: &str) -> String {
        self.wrap("33", text)
    }

    /// Something failed: a request that could not be satisfied.
    pub fn bad(&self, text: &str) -> String {
        self.wrap("31", text)
    }
}

/// `NO_COLOR` set to anything non-empty, or a terminal that cannot render it.
fn suppressed_by_environment() -> bool {
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let dumb = std::env::var_os("TERM").is_some_and(|v| v == "dumb");
    no_color || dumb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_style_adds_nothing() {
        let s = Style::plain();
        assert_eq!(s.bold("x"), "x");
        assert_eq!(s.prefix("10.0.0.0/8"), "10.0.0.0/8");
    }

    #[test]
    fn always_wraps_in_escapes() {
        let s = Style::new(When::Always);
        assert_eq!(s.bold("x"), "\x1b[1mx\x1b[0m");
        assert_eq!(s.dim("x"), "\x1b[2mx\x1b[0m");
        assert_eq!(s.prefix("x"), "\x1b[36mx\x1b[0m");
        assert_eq!(s.title("x"), "\x1b[1;36mx\x1b[0m");
        assert_eq!(s.good("x"), "\x1b[32mx\x1b[0m");
        assert_eq!(s.warn("x"), "\x1b[33mx\x1b[0m");
        assert_eq!(s.bad("x"), "\x1b[31mx\x1b[0m");
    }

    #[test]
    fn never_wins_over_a_terminal() {
        assert_eq!(Style::new(When::Never).bold("x"), "x");
        assert_eq!(Style::for_stderr(When::Never).bad("x"), "x");
    }

    #[test]
    fn always_wins_over_no_color() {
        // An explicit --color=always is the user asking for it directly, so it
        // does not consult the environment at all.
        assert_eq!(Style::new(When::Always).bold("x"), "\x1b[1mx\x1b[0m");
    }

    #[test]
    fn auto_is_off_when_not_a_terminal() {
        // The test harness captures stdout, so this is never a terminal.
        assert_eq!(Style::new(When::Auto).bold("x"), "x");
    }

    #[test]
    fn every_sequence_is_closed() {
        let s = Style::new(When::Always);
        for painted in [
            s.bold("a"),
            s.dim("a"),
            s.prefix("a"),
            s.good("a"),
            s.warn("a"),
            s.bad("a"),
        ] {
            assert!(
                painted.ends_with("\x1b[0m"),
                "{painted:?} left the terminal styled"
            );
            assert_eq!(painted.matches('\x1b').count(), 2, "{painted:?}");
        }
    }
}
