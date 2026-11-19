use super::*;

#[test]
fn request_metadata_keeps_the_origin_and_colons_in_session_names() {
    assert_eq!(
        route(Some("canix-pinentry-v1:zellij:17:work:remote")),
        Route::Zellij {
            pane: 17,
            session: "work:remote".into(),
        }
    );
}

#[test]
fn agents_require_explicit_desktop_requests() {
    for data in [
        None,
        Some("other-data"),
        Some("canix-pinentry-v1:tty"),
        Some("canix-pinentry-v1:zellij:bad:main"),
        Some("canix-pinentry-v1:zellij:1:"),
        Some("canix-pinentry-v1:zellij:1:main\nother"),
        Some("canix-pinentry-v1:unknown"),
    ] {
        assert_eq!(route(data), Route::Tty);
    }
    assert_eq!(route(Some("canix-pinentry-v1:desktop")), Route::Desktop);
}

#[test]
fn malformed_desktop_snapshots_fail_closed() {
    for data in [
        "canix-pinentry-v1:desktop:00::::", // NUL cannot enter exec's environment.
        "canix-pinentry-v1:desktop:6::::",  // Truncated hex.
        "canix-pinentry-v1:desktop:zz::::",
        "canix-pinentry-v1:desktop::::",    // Missing field.
        "canix-pinentry-v1:desktop:::::::", // Extra field.
    ] {
        assert_eq!(route(Some(data)), Route::Tty);
    }
}

#[test]
fn older_desktop_snapshots_clear_the_agent_qt_override() {
    let data = "canix-pinentry-v1:desktop:3a30:::783131:2f72756e2f757365722f31303030";
    assert_eq!(route(Some(data)), Route::Desktop);
    let context = desktop_context(data).unwrap();
    assert_eq!(context[0], Some(OsString::from(":0")));
    assert_eq!(context[5], None);
}

#[test]
fn live_zellij_beats_stale_desktop_context_even_over_ssh() {
    assert_eq!(
        client_route(
            Some("canix-pinentry-v1:desktop"),
            Some("17"),
            Some("main"),
            true,
            true,
            true
        ),
        Route::Zellij {
            pane: 17,
            session: "main".into()
        }
    );
}

#[test]
fn explicit_terminal_context_survives_background_clients() {
    assert_eq!(
        client_route(
            Some("canix-pinentry-v1:zellij:2:main"),
            None,
            None,
            false,
            false,
            false
        ),
        Route::Zellij {
            pane: 2,
            session: "main".into()
        }
    );
    assert_eq!(
        client_route(Some("canix-pinentry-v1:tty"), None, None, false, true, true),
        Route::Tty
    );
}

#[test]
fn ssh_and_headless_clients_do_not_inherit_desktop_routing() {
    for (ssh, graphical) in [(false, false), (true, false), (true, true)] {
        assert_eq!(
            client_route(
                Some("canix-pinentry-v1:desktop"),
                None,
                None,
                ssh,
                graphical,
                true
            ),
            Route::Tty
        );
    }
    assert_eq!(
        client_route(None, None, None, false, true, true),
        Route::Desktop
    );
}

#[test]
fn rewrites_gpg_and_rage_terminal_options_without_changing_other_protocol_bytes() {
    for option in [
        "OPTION ttyname=/dev/pts/1\n",
        "OPTION ttyname=/dev/tty\n",
        "OPTION ttyname = /dev/pts/1\r\n",
        "option --ttyname /dev/pts/1\n",
    ] {
        assert_eq!(
            request_line(option.as_bytes(), Some("/dev/pts/42"), false),
            b"OPTION ttyname=/dev/pts/42\n"
        );
    }
    for line in [
        b"SETDESC PIN%25%0A\r\n".as_slice(),
        b"OPTION ttytype=xterm\n",
        b"BYE\n",
        b"D \xff\n",
    ] {
        assert_eq!(request_line(line, Some("/dev/pts/42"), false), line);
    }
}

#[test]
fn clients_without_display_or_terminal_are_headless() {
    assert_eq!(
        client_route(None, None, None, false, false, false),
        Route::Headless
    );
    assert_eq!(
        client_route(None, None, None, true, true, false),
        Route::Headless
    );
    // A graphical local client keeps Qt; an inherited marker still wins.
    assert_eq!(
        client_route(None, None, None, false, true, false),
        Route::Desktop
    );
    assert_eq!(
        client_route(
            Some("canix-pinentry-v1:tty"),
            None,
            None,
            false,
            false,
            false
        ),
        Route::Tty
    );
    assert_eq!(route(Some(&Route::Headless.user_data())), Route::Headless);
}

#[test]
fn forwarded_requests_drop_the_requesting_hosts_terminal_and_display() {
    for option in [
        "OPTION ttyname=/dev/pts/1\n",
        "OPTION display=:1\n",
        "OPTION xauthority=/run/user/1000/xauth\n",
        "OPTION touch-file=/run/user/1000/gnupg/S.gpg-agent\n",
    ] {
        assert_eq!(request_line(option.as_bytes(), None, true), b"NOP\n");
    }
    assert_eq!(
        request_line(b"OPTION ttyname=/dev/pts/1\n", Some("/dev/pts/42"), true),
        b"OPTION ttyname=/dev/pts/42\n"
    );
    assert_eq!(
        request_line(b"OPTION display=:1\n", Some("/dev/pts/42"), true),
        b"NOP\n"
    );
    for line in [
        b"OPTION ttytype=xterm\n".as_slice(),
        b"OPTION lc-ctype=C.UTF-8\n",
        b"GETPIN\n",
    ] {
        assert_eq!(request_line(line, None, true), line);
    }
    // Local requests keep their display options untouched.
    assert_eq!(
        request_line(b"OPTION display=:1\n", Some("/dev/pts/42"), false),
        b"OPTION display=:1\n"
    );
}

#[test]
fn only_an_active_unlocked_local_graphical_seat_is_in_use() {
    let seat = "Remote=no\nType=wayland\nActive=yes\nLockedHint=no\n";
    assert!(seat_in_use(seat));
    for (from, to) in [
        ("LockedHint=no", "LockedHint=yes"),
        ("Active=yes", "Active=no"),
        ("Remote=no", "Remote=yes"),
        ("Type=wayland", "Type=tty"),
        ("LockedHint=no\n", ""),
    ] {
        assert!(!seat_in_use(&seat.replace(from, to)), "{to}");
    }
}

#[test]
fn attached_clients_yield_their_focused_terminal_pane() {
    let clients = "CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n1         plugin_2       N/A\n2         terminal_17    nu\n";
    assert_eq!(client_pane(clients), Some(17));
    assert_eq!(
        client_pane("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n"),
        None
    );
}

#[test]
fn peers_are_tab_separated_commands() {
    assert_eq!(
        parse_peers("ssh\t-o\tBatchMode=yes\tcan@peer\n\n\t\nlocal router\n"),
        [
            vec!["ssh", "-o", "BatchMode=yes", "can@peer"],
            vec!["local router"],
        ]
    );
}

#[test]
fn only_a_complete_ok_status_line_accepts_a_peer() {
    for greeting in [b"OK\n".as_slice(), b"OK Pleased to meet you\n"] {
        assert!(accepted(greeting));
    }
    for greeting in [
        b"OKAY welcome\n".as_slice(),
        b"OK",
        b"OK Pleased",
        b"ERR 83886179 locked\n",
        b"",
    ] {
        assert!(!accepted(greeting));
    }
}

#[test]
fn forwarded_descriptions_name_the_requesting_host() {
    assert_eq!(
        describe_requester(b"SETDESC Please unlock the card%0A\n", Some("atlas")),
        b"SETDESC Request from atlas%0APlease unlock the card%0A\n"
    );
    for (line, host) in [
        (b"SETPROMPT PIN\n".as_slice(), Some("atlas")),
        (b"SETDESC text\n", None),
    ] {
        assert_eq!(describe_requester(line, host), line);
    }
    assert_eq!(
        request_line(b"OPTION allow-external-password-cache\n", None, true),
        b"NOP\n"
    );
}

#[test]
fn a_description_at_the_assuan_line_limit_stays_unlabeled() {
    let full = [b"SETDESC ".as_slice(), &[b'x'; 990], b"\n"].concat();
    assert_eq!(describe_requester(&full, Some("atlas")), full);
}
