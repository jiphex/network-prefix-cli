//! End-to-end tests for fabrictool: run the real binary and read its output,
//! which is the only place argument arrangement and exit codes are actually
//! exercised.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fabrictool"))
        .args(args)
        .output()
        .expect("binary runs")
}

/// Remove ANSI SGR sequences, so styled and unstyled output can be compared.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        for c in chars.by_ref() {
            if c == 'm' {
                break;
            }
        }
    }
    out
}

fn stdout(args: &[&str]) -> String {
    let out = run(args);
    assert!(out.status.success(), "expected success from {args:?}");
    String::from_utf8(out.stdout).expect("utf-8 output")
}

fn stderr(args: &[&str]) -> String {
    String::from_utf8(run(args).stderr).expect("utf-8 output")
}

#[test]
fn reports_on_a_bare_switch_count() {
    let s = stdout(&["8"]);
    assert!(s.starts_with("8 switches  -  full mesh"), "{s}");
    assert!(s.contains("Links          28"), "{s}");
    assert!(s.contains("7 ports on each switch"), "{s}");
    // Nothing was said about speed, so nothing is claimed about bandwidth.
    assert!(!s.contains("Link speed"), "{s}");
}

#[test]
fn counts_the_optics_a_breakout_saves() {
    let s = stdout(&["8", "@100G", "%400G"]);
    assert!(s.contains("2 x 400G on each switch"), "{s}");
    assert!(s.contains("16  400G transceiver"), "{s}");
    assert!(s.contains("28  duplex coupler"), "{s}");
    // The straight version of the same fabric buys more than three times as
    // many transceivers, which is the entire point of the comparison.
    let straight = stdout(&["8", "@100G"]);
    assert!(straight.contains("56  100G transceiver"), "{straight}");
}

#[test]
fn the_shape_changes_the_counts() {
    assert!(stdout(&["16", "/ring"]).contains("Links          16"));
    assert!(stdout(&["16", "/star"]).contains("Links          15"));
    assert!(stdout(&["16"]).contains("Links          120"));
    assert!(stdout(&["16", "x2"]).contains("Links          240"));
}

#[test]
fn flags_and_operators_may_be_interleaved() {
    let a = stdout(&["8", "@100G", "--json"]);
    let b = stdout(&["--json", "8", "@100G"]);
    let c = stdout(&["8", "--json", "@100G"]);
    assert_eq!(a, b);
    assert_eq!(a, c);
    // -n takes a number, and that number is not the switch count.
    let s = stdout(&["-n", "2", "8", ".", "--color=never"]);
    assert!(s.contains("... (showing 2 of 28"), "{s}");
}

#[test]
fn a_schedule_lists_every_link_when_asked() {
    let s = stdout(&["8", ".", "--all", "--color=never"]);
    let patches = s.lines().filter(|l| l.contains("->")).count();
    assert_eq!(patches, 28);
    assert!(s.contains("sw1:1  ->  sw2:1"), "{s}");
    assert!(!s.contains("showing"), "{s}");
}

#[test]
fn quiet_is_a_bill_of_materials_one_item_per_line() {
    let s = stdout(&["8", "@100G", "%400G", "-q"]);
    assert_eq!(
        s,
        "16\ttransceiver-400G\n16\tbreakout-1x4-400G\n28\tcoupler-100G\n16\tport-switch-400G\n"
    );
    // And the schedule when the schedule is what was asked for.
    let s = stdout(&["4", ".", "-q"]);
    assert_eq!(s.lines().count(), 6);
    assert_eq!(s.lines().next(), Some("sw1:1 sw2:1"));
}

#[test]
fn machine_output_is_never_coloured() {
    for args in [
        vec!["8", "@100G", "-q", "--color=always"],
        vec!["8", "@100G", "--json", "--color=always"],
    ] {
        let s = stdout(&args);
        assert!(!s.contains('\x1b'), "{args:?} came out coloured");
    }
}

#[test]
fn colour_never_changes_the_layout() {
    for args in [
        vec!["8", "@100G", "%400G", "=32", "."],
        vec!["9", "@25G", "%100G", "/star", "=8"],
        vec!["4", "/ring", "x2"],
    ] {
        let mut painted = args.clone();
        painted.push("--color=always");
        let mut plain = args.clone();
        plain.push("--color=never");
        assert_eq!(strip_ansi(&stdout(&painted)), stdout(&plain), "{args:?}");
    }
}

#[test]
fn json_is_exact_and_parseable_shaped() {
    let s = stdout(&["8", "@100G", "%400G", "=32", ".", "--json"]);
    for want in [
        "\"switches\": 8",
        "\"links\": 28",
        "\"link_speed_mbps\": 100000",
        "\"port_speed_mbps\": 400000",
        "\"arrangement\": \"split-both-ends\"",
        "\"trunk_ports\": 16",
        "\"total_mbps\": 2800000",
        "\"fits\": true",
    ] {
        assert!(s.contains(want), "{want} missing from {s}");
    }
}

#[test]
fn a_port_budget_is_answered_in_the_report() {
    let s = stdout(&["8", "@100G", "%400G", "=32", "--color=never"]);
    assert!(
        s.contains("Ports on a 32-port switch\n  yes - it fits"),
        "{s}"
    );
    let out = run(&["48", "@100G", "/star", "=32", "--color=never"]);
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("no - the hub is 15 short"), "{s}");
    // Only --quiet turns the question into an exit status; on screen the
    // answer is there to be read.
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn under_quiet_a_port_budget_is_the_exit_status() {
    assert_eq!(
        run(&["8", "%400G", "@100G", "=32", "-q"]).status.code(),
        Some(0)
    );
    assert_eq!(run(&["48", "/star", "=32", "-q"]).status.code(), Some(4));
    // A typo is not a confident no.
    let out = run(&["48", "/star", "=lots", "-q"]);
    assert_eq!(out.status.code(), Some(1));
    assert_ne!(out.status.code(), Some(4), "a typo looked like a no");
}

#[test]
fn a_fabric_that_cannot_be_built_is_not_the_same_as_bad_input() {
    // A splitter DAC into a mesh: coherent, and nothing to plug it into.
    let out = run(&["8", "@100G", "%400G", "--media=dac"]);
    assert_eq!(out.status.code(), Some(3));
    let e = String::from_utf8(out.stderr).unwrap();
    assert!(e.contains("/star"), "{e}");

    // Bad input, by contrast, is 1 all the way down.
    for args in [
        vec!["1"],
        vec!["0"],
        vec!["banana"],
        vec!["8", "@banana"],
        vec!["8", "/banana"],
        vec!["8", "%0"],
        vec!["8", "@40G", "%100G"],
        vec!["8", "@100G", "@400G"],
    ] {
        assert_eq!(run(&args).status.code(), Some(1), "{args:?}");
    }
}

#[test]
fn errors_say_what_to_do_about_it() {
    assert!(stderr(&["8", "%400G"]).contains("@100G"));
    assert!(stderr(&["1"]).contains("not a fabric"));
    assert!(stderr(&["8", "/banana"]).contains("/mesh"));
    // On stderr, prefixed, so it never lands in a pipeline's data - and a
    // run that worked says nothing there at all.
    assert!(stderr(&["1"]).starts_with("fabrictool:"));
    assert!(stderr(&["8"]).is_empty());
}

#[test]
fn the_report_never_argues_with_itself() {
    // Every count in the report is derived from the same plan, so the
    // headline link count and the schedule have to agree, whatever the shape.
    for args in [
        vec!["7", ".", "--all"],
        vec!["7", "/ring", ".", "--all"],
        vec!["7", "/star", ".", "--all"],
        vec!["5", "x3", ".", "--all"],
        vec!["6", "@100G", "%400G", ".", "--all"],
    ] {
        let mut with_colour = args.clone();
        with_colour.push("--color=never");
        let s = stdout(&with_colour);
        let listed = s.lines().filter(|l| l.contains("->")).count();
        let headline = s
            .lines()
            .find(|l| l.trim_start().starts_with("Links "))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|n| n.replace(',', "").parse::<usize>().ok())
            .expect("a link count");
        assert_eq!(listed, headline, "{args:?}");
    }
}
