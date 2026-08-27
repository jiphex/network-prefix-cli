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
        Style::decide(
            when,
            std::io::stdout().is_terminal(),
            suppressed_by_environment(),
        )
    }

    /// For messages on stderr, which is a different stream and may be a
    /// terminal when stdout is not.
    pub fn for_stderr(when: When) -> Style {
        Style::decide(
            when,
            std::io::stderr().is_terminal(),
            suppressed_by_environment(),
        )
    }

    /// The rule itself, with both of its inputs handed in.
    ///
    /// Kept separate from the two constructors above so it can be tested for
    /// what it decides rather than for what the machine running the tests
    /// happens to look like. Asking the real stdout made the answer depend on
    /// how the suite was invoked: piped it said no, and under a terminal - an
    /// interactive `cargo test`, or a Nix builder - it said yes.
    fn decide(when: When, is_terminal: bool, suppressed: bool) -> Style {
        Style {
            on: match when {
                When::Always => true,
                When::Never => false,
                When::Auto => is_terminal && !suppressed,
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
        // Never holds whatever the stream and the environment say.
        assert_eq!(Style::decide(When::Never, true, false).bold("x"), "x");
    }

    #[test]
    fn always_wins_over_no_color() {
        // An explicit --color=always is the user asking for it directly, so it
        // does not consult the environment at all.
        assert_eq!(Style::new(When::Always).bold("x"), "\x1b[1mx\x1b[0m");
    }

    #[test]
    fn the_whole_decision_table() {
        // (when, terminal, suppressed) -> coloured?
        let cases = [
            (When::Auto, false, false, false),
            (When::Auto, true, false, true),
            // NO_COLOR or TERM=dumb beat a terminal.
            (When::Auto, true, true, false),
            (When::Auto, false, true, false),
            // An explicit choice beats everything, in both directions.
            (When::Always, false, true, true),
            (When::Always, true, false, true),
            (When::Never, true, false, false),
            (When::Never, false, false, false),
        ];
        for (when, is_terminal, suppressed, coloured) in cases {
            let got = Style::decide(when, is_terminal, suppressed).bold("x") != "x";
            assert_eq!(
                got, coloured,
                "{when:?} with terminal={is_terminal} suppressed={suppressed}"
            );
        }
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
