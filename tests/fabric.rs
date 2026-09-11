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
    assert!(stdout(&["16", "--shape=ring"]).contains("Links          16"));
    assert!(stdout(&["16", "--shape=star"]).contains("Links          15"));
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
    let s = stdout(&["-n", "2", "8", "--schedule", "--color=never"]);
    assert!(s.contains("... (showing 2 of 28"), "{s}");
}

#[test]
fn a_schedule_lists_every_link_when_asked() {
    let s = stdout(&["8", "--schedule", "--all", "--color=never"]);
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
    let s = stdout(&["4", "--schedule", "-q"]);
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
        vec!["8", "@100G", "%400G", "=32", "--schedule"],
        vec!["9", "@25G", "%100G", "--shape=star", "=8"],
        vec!["4", "--shape=ring", "x2"],
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
    let s = stdout(&["8", "@100G", "%400G", "=32", "--schedule", "--json"]);
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
    let out = run(&["48", "@100G", "--shape=star", "=32", "--color=never"]);
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
    assert_eq!(
        run(&["48", "--shape=star", "=32", "-q"]).status.code(),
        Some(4)
    );
    // A typo is not a confident no.
    let out = run(&["48", "--shape=star", "=lots", "-q"]);
    assert_eq!(out.status.code(), Some(1));
    assert_ne!(out.status.code(), Some(4), "a typo looked like a no");
}

#[test]
fn a_leaf_spine_counts_its_two_populations_apart() {
    let s = stdout(&["16", "+2", "@100G", "%400G", "-48@25G", "--color=never"]);
    assert!(
        s.starts_with("16 leaves + 2 spines  -  leaf-spine at 100G"),
        "{s}"
    );
    assert!(s.contains("Links          32"), "{s}");
    assert!(s.contains("4 x 400G on each spine"), "{s}");
    assert!(s.contains("2 x 100G on each leaf"), "{s}");
    assert!(s.contains("Server ports   48 x 25G on each leaf"), "{s}");
    assert!(s.contains("Per leaf       6:1"), "{s}");
    // A 400G spine port reaches four leaves, so four ports carry sixteen.
    assert!(s.contains("8  400G transceiver"), "{s}");
    assert!(s.contains("32  100G transceiver"), "{s}");
}

/// The arrangement a rack is actually built in: a pair of leaves under a
/// pair of spines, every leaf on every spine.
#[test]
fn a_rack_is_a_pair_of_spines_and_survives_losing_one() {
    let s = stdout(&["2", "+2", "@100G", "-48@25G", "--color=never"]);
    assert!(
        s.starts_with("2 leaves + 2 spines  -  leaf-spine at 100G"),
        "{s}"
    );
    assert!(s.contains("Links          4"), "{s}");
    assert!(s.contains("each leaf 2 links, 200G"), "{s}");
    assert!(
        s.contains("Spine loss     each leaf keeps 1 uplink of 2"),
        "{s}"
    );
    assert!(s.contains("One spine down 12:1"), "{s}");
    // Both leaves reach both spines, and nothing is wired twice.
    let schedule = stdout(&["2", "+2", "--schedule", "--all", "-q"]);
    assert_eq!(
        schedule,
        "spine1:1 leaf1:1\nspine1:2 leaf2:1\nspine2:1 leaf1:2\nspine2:2 leaf2:2\n"
    );
}

#[test]
fn a_leaf_spine_is_a_pair_of_spines_unless_told_otherwise() {
    let s = stdout(&["16", "--shape=leaf-spine", "@100G", "--color=never"]);
    assert!(s.starts_with("16 leaves + 2 spines"), "{s}");
    // One is still allowed, and still says what it is.
    let s = stdout(&["16", "+1", "@100G", "--color=never"]);
    assert!(s.starts_with("16 leaves + 1 spine"), "{s}");
    assert!(s.contains("single point of failure"), "{s}");
}

#[test]
fn uplinks_change_the_ratio_and_nothing_else_about_the_servers() {
    let ratio = |spines: &str| {
        let s = stdout(&["16", spines, "@100G", "-48@25G", "--color=never"]);
        s.lines()
            .find(|l| l.trim_start().starts_with("Per leaf"))
            .map(|l| l.split_whitespace().nth(2).unwrap().to_string())
            .expect("a ratio")
    };
    assert_eq!(ratio("+2"), "6:1");
    assert_eq!(ratio("+4"), "3:1");
    assert_eq!(ratio("+6"), "2:1");
    assert_eq!(ratio("+12"), "1:1");
}

#[test]
fn server_ports_count_against_the_port_budget() {
    // Two uplinks and 48 servers is 50 ports, whatever they are facing.
    let s = stdout(&["16", "+2", "@100G", "-48@25G", "=56", "--color=never"]);
    assert!(
        s.contains("each leaf   50 of 56 ports (2 at 100G, 48 at 25G)"),
        "{s}"
    );
    assert_eq!(
        run(&["16", "+2", "@100G", "-48@25G", "=48", "-q"])
            .status
            .code(),
        Some(4),
        "50 ports do not fit in 48"
    );
    // Arriving on split ports, the same servers take twelve ports, not 48.
    let s = stdout(&["16", "+2", "@100G", "-48@25G%100G", "=56", "--color=never"]);
    assert!(s.contains("each leaf   14 of 56 ports"), "{s}");
}

/// Four 32-port 400G spines, four racks of paired leaves, and leaves whose
/// front panel is 48x25G + 2x200G + 4x100G: which cabling options fit.
#[test]
fn a_real_build_is_checked_against_the_leaf_it_has() {
    let profile = ["=32@400G", "=4@100G", "=2@200G", "=48@25G"];

    // 100G to every spine, out of the spine's 400G ports split four ways.
    let mut args = vec!["8", "+4", "@100G", "%400G", "-48@25G", "--color=never"];
    args.extend(profile);
    let s = stdout(&args);
    assert!(
        s.contains("each spine  2 of 32 ports at 400G, 30 spare"),
        "{s}"
    );
    assert!(
        s.contains("each leaf  4 of 4 ports at 100G, 0 spare"),
        "{s}"
    );
    assert!(
        s.contains("each leaf  48 of 48 ports at 25G, 0 spare"),
        "{s}"
    );
    assert!(!s.contains(" no - "), "{s}");

    // 200G to every spine needs four 200G ports on a leaf that has two, so
    // the full mesh cannot be kept at that speed.
    let mut args = vec!["8", "+4", "@200G", "%400G", "-48@25G", "--color=never"];
    args.extend(profile);
    let s = stdout(&args);
    assert!(
        s.contains("200G ports on a 2-port switch\n  no - each leaf is 2 short"),
        "{s}"
    );
    assert_eq!(
        run(&["8", "+4", "@200G", "%400G", "=2@200G", "-q"])
            .status
            .code(),
        Some(4)
    );
}

/// A speed picks out a kind of switch in most fabrics; where it does not,
/// the question has to be able to say which switches it is about.
#[test]
fn a_budget_can_name_the_switches_it_is_about() {
    // Leaves and spines both at 100G here, so the speed alone cannot separate
    // them: the leaves fit four ports and the spines do not.
    let s = stdout(&["8", "+2", "@100G", "=4@100G:leaf", "--color=never"]);
    assert!(s.contains("100G ports on a 4-port leaf"), "{s}");
    assert!(
        s.contains("each leaf  2 of 4 ports at 100G, 2 spare"),
        "{s}"
    );
    // The report still describes both kinds of switch; it is the answer to
    // the question that is about leaves alone.
    let answer = s
        .split("100G ports on a 4-port leaf")
        .nth(1)
        .expect("the budget section");
    assert!(!answer.contains("each spine"), "{s}");

    let out = run(&["8", "+2", "@100G", "=4@100G:spine", "--color=never"]);
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("100G ports on a 4-port spine"), "{s}");
    assert!(s.contains("no - each spine is 4 short"), "{s}");
    assert_eq!(
        run(&["8", "+2", "@100G", "=4@100G:spine", "-q"])
            .status
            .code(),
        Some(4)
    );

    // A kind of switch the fabric has not got is a misunderstanding of the
    // fabric, not a plan that does not fit, so it is refused.
    let out = run(&["8", "@100G", "=32:leaf"]);
    assert_eq!(out.status.code(), Some(1));
    let e = String::from_utf8(out.stderr).unwrap();
    assert!(e.contains("a full mesh has no leaves"), "{e}");
    assert!(e.contains("it has 8 switches"), "{e}");
    assert!(stderr(&["8", "+2", "@100G", "=4@100G:banana"]).contains("not a kind of switch"));
}

#[test]
fn servers_hang_off_the_edge_of_whatever_shape_it_is() {
    assert!(stdout(&["8", "-24@10G", "@100G", "--color=never"]).contains("on each switch"));
    assert!(
        stdout(&["8", "--shape=star", "-24@10G", "@100G", "--color=never"])
            .contains("on each spoke")
    );
    assert!(stdout(&["8", "+2", "-24@10G", "@100G", "--color=never"]).contains("on each leaf"));
    // Nothing hangs off a fabric nobody mentioned servers for.
    assert!(!stdout(&["8", "@100G"]).contains("Oversubscription"));
}

#[test]
fn the_schedule_names_spines_and_leaves() {
    let s = stdout(&["4", "+2", "--schedule", "--all", "-q"]);
    assert_eq!(
        s,
        "spine1:1 leaf1:1\nspine1:2 leaf2:1\nspine1:3 leaf3:1\nspine1:4 leaf4:1\n\
         spine2:1 leaf1:2\nspine2:2 leaf2:2\nspine2:3 leaf3:2\nspine2:4 leaf4:2\n"
    );
}

/// The drawing is the one output that shows the shape rather than counting
/// it, so what matters is that it is a graph of the right fabric.
#[test]
fn dot_draws_the_fabric() {
    let s = stdout(&["4", "+2", "@100G", "%400G", "-48@25G", "--dot"]);
    assert!(
        s.starts_with("// fabrictool: 4 leaves + 2 spines - leaf-spine at 100G"),
        "{s}"
    );
    assert!(s.contains("subgraph cluster_spine {"), "{s}");
    assert!(
        s.contains("leaf1 [label=\"leaf1\\n2 x 100G\\n48 x 25G servers\"]"),
        "{s}"
    );
    // Every leaf reaches every spine, and each pair gets one edge.
    assert_eq!(s.matches(" -- ").count(), 8);
    // It is never coloured, because it is another program's input.
    let painted = stdout(&["4", "+2", "@100G", "--dot", "--color=always"]);
    assert!(!painted.contains('\x1b'), "{painted}");
    // With --schedule it draws a cable at a time, ports and all.
    let s = stdout(&["4", "+2", "@100G", "%400G", "--dot", "--schedule"]);
    assert!(s.contains("[label=\"spine1:1/1 - leaf1:1\"]"), "{s}");
}

#[test]
fn a_fabric_that_cannot_be_built_is_not_the_same_as_bad_input() {
    // A splitter DAC into a mesh is coherent, and has nothing to plug into.
    let out = run(&["8", "@100G", "%400G", "--media=dac"]);
    assert_eq!(out.status.code(), Some(3));
    let e = String::from_utf8(out.stderr).unwrap();
    assert!(e.contains("--shape=star"), "{e}");

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
        vec!["8", "+2", "--shape=mesh"],
        vec!["8", "+0"],
        vec!["8", "-48"],
        vec!["8", "-48@banana"],
    ] {
        assert_eq!(run(&args).status.code(), Some(1), "{args:?}");
    }
}

#[test]
fn errors_say_what_to_do_about_it() {
    assert!(stderr(&["8", "%400G"]).contains("@100G"));
    assert!(stderr(&["1"]).contains("not a fabric"));
    // A sigil that is really a flag is answered with the flag.
    assert!(stderr(&["8", "/banana"]).contains("--shape=mesh, ring"));
    assert!(stderr(&["8", "/ring"]).contains("the shape is a flag"));
    assert!(stderr(&["8", "."]).contains("the patch schedule is --schedule"));
    // On stderr, prefixed, so it never reaches a pipeline's data - and a
    // run that worked says nothing there at all.
    assert!(stderr(&["1"]).starts_with("fabrictool:"));
    assert!(stderr(&["8"]).is_empty());
}

#[test]
fn the_report_never_argues_with_itself() {
    // Every count in the report is derived from the same plan, so the
    // headline link count and the schedule have to agree, whatever the shape.
    for args in [
        vec!["7", "--schedule", "--all"],
        vec!["7", "--shape=ring", "--schedule", "--all"],
        vec!["7", "--shape=star", "--schedule", "--all"],
        vec!["5", "x3", "--schedule", "--all"],
        vec!["6", "@100G", "%400G", "--schedule", "--all"],
        vec!["6", "+2", "--schedule", "--all"],
        vec![
            "6",
            "+3",
            "@100G",
            "%400G",
            "-24@10G",
            "--schedule",
            "--all",
        ],
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
