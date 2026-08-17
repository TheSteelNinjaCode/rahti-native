//! The native command allowlist, and the URL check that keeps
//! `open_external` from being a way to run things.

use crate::capabilities::*;

#[test]
fn the_allowlist_is_the_five_it_says_it_is() {
    let names: Vec<&str> = commands().iter().map(|c| c.name).collect();
    assert_eq!(
        names,
        vec![
            "platform",
            "app_version",
            "app_data_dir",
            "open_external",
            "choose_file",
        ]
    );
}

#[test]
fn nothing_in_the_allowlist_runs_a_program_or_reads_a_named_path() {
    // The rule, stated as a test so that adding one has to argue with it: an
    // XSS in a native shell reaches whatever the bridge reaches.
    for command in commands() {
        for forbidden in [
            "execute",
            "exec",
            "shell",
            "spawn",
            "process",
            "command",
            "eval",
            "read_file",
            "write_file",
            "fs",
            "remove",
        ] {
            assert!(
                !command.name.contains(forbidden),
                "`{}` is not a command to expose to a page",
                command.name
            );
        }
    }
}

#[test]
fn a_command_that_is_not_on_the_list_is_not_allowed() {
    assert!(is_allowed("platform"));
    assert!(is_allowed("choose_file"));

    for absent in [
        "",
        "Platform",
        "platform ",
        "execute_process",
        "read_file",
        "plugin:shell|execute",
        "app_data_dir\0",
    ] {
        assert!(!is_allowed(absent), "`{absent}` was allowed");
    }
}

#[test]
fn every_command_has_a_distinct_capability_name() {
    let mut seen = std::collections::HashSet::new();
    for command in commands() {
        assert!(
            seen.insert(command.capability),
            "`{}` is claimed twice",
            command.capability
        );
        assert!(!command.summary.is_empty());
    }
    assert_eq!(capability_names().len(), commands().len());
}

#[test]
fn only_the_three_schemes_that_mean_somewhere_else_open_externally() {
    for good in [
        "https://example.com",
        "http://example.com/docs?q=1",
        "HTTPS://Example.COM",
        "mailto:ada@example.test",
    ] {
        assert!(is_external_url(good), "{good} was refused");
    }
}

#[test]
fn a_file_url_never_opens_externally() {
    // On Windows the shell opens whatever the extension is associated with,
    // which includes executables and script hosts.
    for bad in [
        "file:///C:/Windows/System32/cmd.exe",
        "FILE:///etc/passwd",
        "file://server/share/setup.exe",
    ] {
        assert!(!is_external_url(bad), "{bad} was allowed");
    }
}

#[test]
fn a_scheme_that_reenters_a_privileged_context_is_refused() {
    for bad in [
        "javascript:alert(1)",
        "JavaScript:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        "vbscript:msgbox",
        "ms-msdt:/id",
        "tauri://localhost",
        "intent://scan#Intent;scheme=zxing;end",
    ] {
        assert!(!is_external_url(bad), "{bad} was allowed");
    }
}

#[test]
fn a_url_that_is_not_a_url_is_refused() {
    for bad in [
        "",
        "   ",
        "example.com",
        "http:",
        "https:",
        "http://",
        "https:///etc",
        "mailto:",
        "https://example.com\r\nX-Injected: 1",
        "https://example.com\u{0}",
    ] {
        assert!(!is_external_url(bad), "{bad:?} was allowed");
    }
}
