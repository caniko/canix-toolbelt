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
        "canix-pinentry-v1:desktop::::",   // Missing field.
        "canix-pinentry-v1:desktop::::::", // Extra field.
    ] {
        assert_eq!(route(Some(data)), Route::Tty);
    }
}

#[test]
fn live_zellij_beats_stale_desktop_context_even_over_ssh() {
    assert_eq!(
        client_route(
            Some("canix-pinentry-v1:desktop"),
            Some("17"),
            Some("main"),
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
            false
        ),
        Route::Zellij {
            pane: 2,
            session: "main".into()
        }
    );
    assert_eq!(
        client_route(Some("canix-pinentry-v1:tty"), None, None, false, true),
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
                graphical
            ),
            Route::Tty
        );
    }
    assert_eq!(client_route(None, None, None, false, true), Route::Desktop);
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
            request_line(option.as_bytes(), "/dev/pts/42"),
            b"OPTION ttyname=/dev/pts/42\n"
        );
    }
    for line in [
        b"SETDESC PIN%25%0A\r\n".as_slice(),
        b"OPTION ttytype=xterm\n",
        b"BYE\n",
        b"D \xff\n",
    ] {
        assert_eq!(request_line(line, "/dev/pts/42"), line);
    }
}
