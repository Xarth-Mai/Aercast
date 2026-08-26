use super::*;

#[test]
fn a_reported_audio_failure_is_not_a_cleanup_failure() {
    assert_eq!(
        stop_result(
            Err(AudioFailure::Media("reported media failure".to_owned())),
            true,
        ),
        Ok(())
    );
    assert_eq!(
        stop_result(
            Err(AudioFailure::Cleanup("cleanup failure".to_owned())),
            true,
        ),
        Err("cleanup failure".to_owned())
    );

    let mut failure = Some(AudioFailure::Media("reported media failure".to_owned()));
    record_cleanup_failure(&mut failure, "cleanup failure".to_owned());
    assert_eq!(
        stop_result(Err(failure.unwrap()), true),
        Err("cleanup failure".to_owned())
    );

    let mut failure = Some(AudioFailure::Cleanup("first cleanup failure".to_owned()));
    record_cleanup_failure(&mut failure, "later cleanup failure".to_owned());
    assert_eq!(
        stop_result(Err(failure.unwrap()), true),
        Err("first cleanup failure".to_owned())
    );
}

#[test]
fn passive_link_contract_rejects_missing_property_error_and_wrong_endpoint() {
    let expected = Endpoints {
        output_node: 1,
        output_port: 2,
        input_node: 3,
        input_port: 4,
    };
    let good = ObservedLink {
        id: 5,
        serial: Some(6),
        endpoints: expected,
        passive: Some(true),
        status: Some(LinkStatus::Active),
    };
    assert!(link_matches(expected, good));
    for bad in [
        ObservedLink {
            serial: None,
            ..good
        },
        ObservedLink {
            passive: Some(false),
            ..good
        },
        ObservedLink {
            passive: None,
            ..good
        },
        ObservedLink {
            status: Some(LinkStatus::Error),
            ..good
        },
        ObservedLink {
            status: None,
            ..good
        },
        ObservedLink {
            endpoints: Endpoints {
                input_port: 5,
                ..expected
            },
            ..good
        },
    ] {
        assert!(!link_matches(expected, bad));
    }
}

#[test]
fn capture_requires_its_own_active_route_to_an_audio_sink() {
    let sinks = HashSet::from([10]);
    let game_route = Endpoints {
        output_node: 1,
        output_port: 2,
        input_node: 10,
        input_port: 11,
    };
    for (endpoints, status, node, port, expected) in [
        (game_route, Some(LinkStatus::Active), 1, 2, true),
        (game_route, Some(LinkStatus::Active), 3, 2, false),
        (
            Endpoints {
                input_node: 20,
                ..game_route
            },
            Some(LinkStatus::Active),
            1,
            2,
            false,
        ),
        (game_route, Some(LinkStatus::Pending), 1, 2, false),
        (game_route, Some(LinkStatus::Error), 1, 2, false),
        (game_route, None, 1, 2, false),
        (game_route, Some(LinkStatus::Active), 1, 3, false),
    ] {
        assert_eq!(
            active_sink_route(endpoints, status, node, port, 20, &sinks),
            expected
        );
    }
}

#[test]
fn unsafe_stream_overrides_and_audio_policy_are_checked() {
    let mut properties = stream_properties();
    assert!(validate_stream_properties(properties.dict()).is_ok());
    assert!(validate_exported_node(properties.dict()).is_ok());
    properties.insert("target.object", "unexpected-source");
    assert!(validate_stream_properties(properties.dict()).is_err());
    properties.insert("node.driver", "true");
    assert!(validate_exported_node(properties.dict()).is_err());

    let mut properties = pw::properties::properties! {
        *pw::keys::APP_ID => "org.example.Game",
        *pw::keys::APP_PROCESS_BINARY => "game-bin",
        *pw::keys::APP_NAME => "Example Game",
        *pw::keys::APP_PROCESS_ID => "1234",
    };
    let normal = playback_policy(properties.dict());
    assert_eq!(normal.identity.as_deref(), Some("org.example.Game"));
    properties.insert(*pw::keys::APP_PROCESS_ID, "9999");
    assert_eq!(normal, playback_policy(properties.dict()));
    let binary_fallback = pw::properties::properties! {
        *pw::keys::APP_PROCESS_BINARY => "game-bin",
        *pw::keys::APP_NAME => "Example Game",
    };
    assert_eq!(
        playback_identity(binary_fallback.dict()).as_deref(),
        Some("game-bin")
    );
    let name_fallback = pw::properties::properties! {
        *pw::keys::APP_NAME => "Example Game",
        *pw::keys::APP_PROCESS_ID => "5678",
    };
    assert_eq!(
        playback_identity(name_fallback.dict()).as_deref(),
        Some("Example Game")
    );
    let communication = pw::properties::properties! {
        *pw::keys::APP_ID => "org.example.Game",
        *pw::keys::MEDIA_ROLE => "Communication",
        *pw::keys::APP_PROCESS_ID => "42",
    };
    let communication = playback_policy(communication.dict());
    assert!(excluded(&communication, &[]));
    assert!(!excluded(&normal, &["Example Game".to_owned()]));
    assert!(excluded(&normal, &["org.example.Game".to_owned()]));
    assert!(!excluded(&normal, &[]));
    assert!(excluded(
        &PlaybackPolicy {
            identity: Some("org.aercast.Aercast".to_owned()),
            communication: false,
        },
        &[],
    ));
    assert!(vanished_object(42, -2));
    assert!(!vanished_object(pw::core::PW_ID_CORE, -2));
    assert!(!vanished_object(42, -13));
}
