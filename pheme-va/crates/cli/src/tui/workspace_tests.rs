use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{mpsc, Arc};

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::tui::app::{ActiveModel, TelemetryTab, NAVIGATION_HISTORY_LIMIT};
use crate::tui::config::TuiConfig;
use crate::tui::events::WorkerCommand;
use crate::tui::model_catalog::{CatalogEntry, ModelCatalog};
use crate::tui::TuiOptions;

fn press(app: &mut App, code: KeyCode) {
    app.handle_terminal_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
        .unwrap();
}

fn entry(id: &str, purpose: &str, available: bool) -> CatalogEntry {
    let family = if purpose == "reply" {
        "qwen2"
    } else {
        "whisper"
    };
    let manifest = toml::from_str(&format!(
        "id='{id}'\nfamily='{family}'\npurpose='{purpose}'\nmodel='fixture.bin'\ndownload_id='fixture-target'\nrepository='org/repo'\nrevision='{}'\nsha256='{}'",
        "a".repeat(40), "b".repeat(64),
    )).unwrap();
    CatalogEntry {
        manifest,
        model_path: "fixture.bin".into(),
        missing_paths: if available {
            vec![]
        } else {
            vec!["fixture.bin".into()]
        },
        adapter_compiled: true,
    }
}

#[test]
fn alphabet_keys_navigate_and_digits_only_control_telemetry() {
    let mut app = crate::tui::app::tests::ready_app();
    app.screen = Screen::Web;
    app.voice.target = Some("invalid URL".into());
    for (key, screen) in [
        ('w', Screen::Web),
        ('b', Screen::Bench),
        ('m', Screen::Models),
    ] {
        press(&mut app, KeyCode::Char(key));
        assert_eq!(app.screen, screen);
        for digit in '1'..='4' {
            press(&mut app, KeyCode::Char(digit));
            assert_eq!(app.screen, screen);
        }
    }
    press(&mut app, KeyCode::Char('t'));
    assert_eq!(app.screen, Screen::Telemetry);
    for (key, tab) in [
        ('1', TelemetryTab::Overview),
        ('2', TelemetryTab::Metrics),
        ('3', TelemetryTab::Runs),
        ('4', TelemetryTab::Logs),
    ] {
        press(&mut app, KeyCode::Char(key));
        assert_eq!(app.screen, Screen::Telemetry);
        assert_eq!(app.telemetry_tab, tab);
    }
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Models);
    press(&mut app, KeyCode::Char('b'));
    assert_eq!(app.screen, Screen::Bench);
    press(&mut app, KeyCode::Char('t'));
    press(&mut app, KeyCode::Char('w'));
    assert_eq!(app.screen, Screen::Web);
}

#[test]
fn standalone_back_uses_welcome_or_bench_and_web_is_accessible() {
    let mut app = crate::tui::app::tests::ready_app();
    press(&mut app, KeyCode::Char('m'));
    assert_eq!(app.screen, Screen::Models);
    press(&mut app, KeyCode::Char('b'));
    assert_eq!(app.screen, Screen::Bench);
    app.active_model = Some(ActiveModel {
        id: "fixture".into(),
        family: "whisper".into(),
        backend: "fake".into(),
        runtime: None,
    });
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Bench);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Models);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Telemetry);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Bench);
    press(&mut app, KeyCode::Char('w'));
    assert_eq!(app.screen, Screen::Web);
    press(&mut app, KeyCode::Char('b'));
    assert_eq!(app.screen, Screen::Bench);
}

#[test]
fn web_back_restores_the_actual_bench_or_models_origin() {
    for origin in [Screen::Bench, Screen::Models] {
        let mut app = crate::tui::app::tests::ready_app();
        app.screen = origin;
        press(&mut app, KeyCode::Char('w'));
        assert_eq!(app.navigation_history, vec![origin]);
        for _ in 0..3 {
            press(&mut app, KeyCode::Char('w'));
        }
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.screen, origin);
        assert!(app.navigation_history.is_empty());
        assert!(!app.should_quit);
    }
}

#[test]
fn nested_pages_unwind_in_order_and_current_page_shortcuts_are_noops() {
    let mut app = crate::tui::app::tests::ready_app();
    app.voice.target = Some("invalid URL".into());
    app.screen = Screen::Web;
    for (key, screen) in [
        ('w', Screen::Web),
        ('b', Screen::Bench),
        ('m', Screen::Models),
        ('w', Screen::Web),
        ('t', Screen::Telemetry),
    ] {
        press(&mut app, KeyCode::Char(key));
        assert_eq!(app.screen, screen);
        let history = app.navigation_history.clone();
        app.scroll = 7;
        app.error_message = Some("retained on same-page shortcut".into());
        app.telemetry_tab = TelemetryTab::Runs;
        press(&mut app, KeyCode::Char(key));
        assert_eq!(app.navigation_history, history);
        assert_eq!(app.scroll, 7);
        assert!(app.error_message.is_some());
        assert_eq!(app.telemetry_tab, TelemetryTab::Runs);
    }
    press(&mut app, KeyCode::Char('?'));
    assert_eq!(app.screen, Screen::Help);
    for expected in [
        Screen::Telemetry,
        Screen::Web,
        Screen::Models,
        Screen::Bench,
        Screen::Web,
    ] {
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.screen, expected);
    }
    for _ in 0..3 {
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.screen, Screen::Bench);
        assert!(app.navigation_history.is_empty());
        assert!(!app.should_quit);
    }
    press(&mut app, KeyCode::Char('?'));
    press(&mut app, KeyCode::Char('?'));
    assert_eq!(app.screen, Screen::Bench);
    assert!(app.navigation_history.is_empty());
}

#[test]
fn page_history_is_bounded_and_root_back_does_not_start_a_new_history() {
    let mut app = crate::tui::app::tests::ready_app();
    app.voice.target = Some("invalid URL".into());
    app.screen = Screen::Web;
    let mut visited = Vec::new();
    for index in 0..NAVIGATION_HISTORY_LIMIT + 9 {
        visited.push(app.screen);
        press(
            &mut app,
            KeyCode::Char(if index % 2 == 0 { 'm' } else { 'w' }),
        );
        assert!(app.navigation_history.len() <= NAVIGATION_HISTORY_LIMIT);
    }
    assert_eq!(
        app.navigation_history,
        visited[visited.len() - NAVIGATION_HISTORY_LIMIT..]
    );
    for expected in visited.into_iter().rev().take(NAVIGATION_HISTORY_LIMIT) {
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.screen, expected);
    }
    for _ in 0..3 {
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.screen, Screen::Bench);
        assert!(app.navigation_history.is_empty());
    }
}

#[test]
fn connected_and_standalone_roots_only_quit_explicitly() {
    for (connected, loaded, initial, root) in [
        (true, false, Screen::Web, Screen::Models),
        (true, false, Screen::ServerTests, Screen::Models),
        (true, false, Screen::Models, Screen::Models),
        (false, false, Screen::Welcome, Screen::Models),
        (false, false, Screen::Models, Screen::Models),
        (false, false, Screen::Web, Screen::Models),
        (false, true, Screen::Bench, Screen::Bench),
        (false, true, Screen::Models, Screen::Bench),
        (false, true, Screen::Web, Screen::Bench),
    ] {
        let mut app = crate::tui::app::tests::ready_app();
        app.voice.target = connected.then(|| "invalid URL".into());
        app.active_model = loaded.then(|| ActiveModel {
            id: "fixture".into(),
            family: "whisper".into(),
            backend: "fake".into(),
            runtime: None,
        });
        app.screen = initial;
        for _ in 0..3 {
            press(&mut app, KeyCode::Esc);
            assert_eq!(app.screen, root);
            assert!(app.navigation_history.is_empty());
            assert!(!app.should_quit);
        }
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit);
    }
}

#[test]
fn web_editor_and_discard_confirmation_close_before_leaving_web() {
    let mut app = crate::tui::app::tests::test_app();
    app.screen = Screen::Models;
    app.voice
        .inspect(Ok(crate::tui::voice::tests::snapshot("web", "partial")));
    press(&mut app, KeyCode::Char('w'));
    let history = app.navigation_history.clone();
    press(&mut app, KeyCode::Char('e'));
    press(&mut app, KeyCode::Char('!'));
    app.voice.editor.as_mut().unwrap().pending = true;
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Web);
    assert_eq!(app.navigation_history, history);
    assert_eq!(app.voice.editor.as_ref().unwrap().buffer.text, "original!");
    assert!(app.voice.editor.as_ref().unwrap().pending);
    press(&mut app, KeyCode::Char('x'));
    assert!(app.voice.discard_editor);
    press(&mut app, KeyCode::Esc);
    assert!(!app.voice.discard_editor);
    assert_eq!(app.screen, Screen::Web);
    assert_eq!(app.navigation_history, history);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Models);
    assert!(app.voice.editor.as_ref().unwrap().pending);
    assert_eq!(
        app.voice
            .snapshot
            .as_ref()
            .unwrap()
            .current_turn
            .as_ref()
            .unwrap()
            .turn_id,
        "web"
    );
}

#[test]
fn tests_text_focus_then_diagnostics_close_before_page_back_without_cancelling() {
    let mut app = crate::tui::app::tests::ready_app();
    app.voice.target = Some("invalid URL".into());
    app.screen = Screen::Models;
    press(&mut app, KeyCode::Char('b'));
    press(&mut app, KeyCode::Char('s'));
    press(&mut app, KeyCode::Char('i'));
    press(&mut app, KeyCode::Char('w'));
    let history = app.navigation_history.clone();
    let request = app.voice.tests.begin("generating");
    app.voice.tests.diagnostics = true;
    press(&mut app, KeyCode::Esc);
    assert!(!app.voice.tests.editing);
    assert!(app.voice.tests.diagnostics);
    assert_eq!(app.voice.tests.buffer.text, "w");
    assert_eq!(app.screen, Screen::ServerTests);
    assert_eq!(app.navigation_history, history);
    press(&mut app, KeyCode::Esc);
    assert!(!app.voice.tests.diagnostics);
    assert_eq!(app.screen, Screen::ServerTests);
    assert_eq!(app.navigation_history, history);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Bench);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Models);
    assert_eq!(app.voice.tests.request, Some(request));
    assert_eq!(app.voice.tests.stage, "generating");
}

#[test]
fn model_subviews_cancel_locally_without_consuming_page_history() {
    use crate::tui::model_actions::RoleSelection;

    let mut app = crate::tui::app::tests::test_app();
    app.screen = Screen::Web;
    press(&mut app, KeyCode::Char('m'));
    let history = app.navigation_history.clone();
    press(&mut app, KeyCode::Char('/'));
    press(&mut app, KeyCode::Char('w'));
    press(&mut app, KeyCode::Esc);
    assert!(!app.models.filtering);
    assert_eq!(app.models.group().filter, "w");
    assert_eq!(app.navigation_history, history);
    app.models.confirmation = Some(Confirmation::Download("fixture".into()));
    press(&mut app, KeyCode::Esc);
    assert!(app.models.confirmation.is_none());
    assert!(app.download.is_none());
    assert_eq!(app.navigation_history, history);
    app.models.role_selection = Some(RoleSelection {
        model_id: "reply".into(),
        directory: "fixture-roles".into(),
        entries: vec![],
        selected_files: vec![],
        cursor: 0,
        adding_path: true,
        path_input: "unsaved.txt".into(),
        error: None,
    });
    press(&mut app, KeyCode::Esc);
    assert!(!app.models.role_selection.as_ref().unwrap().adding_path);
    assert_eq!(app.navigation_history, history);
    press(&mut app, KeyCode::Esc);
    assert!(app.models.role_selection.is_none());
    assert!(app.config.reply_role_files.is_empty());
    assert_eq!(app.navigation_history, history);
    app.models.preview = Some(va_core::LoadedPrompt {
        path: "fixture.txt".into(),
        text: "fixture role".into(),
        sha256: "fixture checksum".into(),
    });
    press(&mut app, KeyCode::Esc);
    assert!(app.models.preview.is_none());
    assert_eq!(app.navigation_history, history);
    press(&mut app, KeyCode::Char('g'));
    press(&mut app, KeyCode::Esc);
    assert!(!app.models.command);
    assert_eq!(app.screen, Screen::Models);
    assert_eq!(app.navigation_history, history);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Web);
}

#[test]
fn telemetry_help_filter_confirmation_and_details_unwind_inside_the_page() {
    use crate::tui::app::tests::{metric, report};

    let mut app = crate::tui::app::tests::test_app();
    app.screen = Screen::Models;
    app.telemetry.push(metric("run", "cpu"));
    app.telemetry
        .upsert_report(report("run", vec![metric("run", "cpu")]));
    press(&mut app, KeyCode::Char('t'));
    let history = app.navigation_history.clone();
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Enter);
    assert!(app.metric_detail);
    press(&mut app, KeyCode::Char('?'));
    assert_eq!(app.screen, Screen::Help);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Telemetry);
    assert!(app.metric_detail);
    assert_eq!(app.navigation_history, history);
    press(&mut app, KeyCode::Char('/'));
    press(&mut app, KeyCode::Char('w'));
    press(&mut app, KeyCode::Esc);
    assert!(!app.filter_editing);
    assert!(app.metric_detail);
    assert_eq!(app.navigation_history, history);
    app.clear_runs_pending = true;
    press(&mut app, KeyCode::Esc);
    assert!(!app.clear_runs_pending);
    assert!(app.metric_detail);
    assert_eq!(app.navigation_history, history);
    press(&mut app, KeyCode::Esc);
    assert!(!app.metric_detail);
    assert_eq!(app.navigation_history, history);
    press(&mut app, KeyCode::Char('x'));
    press(&mut app, KeyCode::Char('3'));
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Enter);
    assert!(app.run_detail && app.historical_detail);
    press(&mut app, KeyCode::Char('?'));
    press(&mut app, KeyCode::Esc);
    assert!(app.run_detail && app.historical_detail);
    assert_eq!(app.navigation_history, history);
    press(&mut app, KeyCode::Esc);
    assert!(app.run_detail && !app.historical_detail);
    assert_eq!(app.navigation_history, history);
    press(&mut app, KeyCode::Esc);
    assert!(!app.run_detail);
    assert_eq!(app.screen, Screen::Telemetry);
    assert_eq!(app.navigation_history, history);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Models);
}

#[test]
fn folder_directory_input_and_help_return_to_their_actual_openers() {
    for connected in [false, true] {
        let mut app = crate::tui::app::tests::ready_app();
        app.voice.target = connected.then(|| "invalid URL".into());
        app.screen = Screen::Web;
        app.active_model = Some(ActiveModel {
            id: "fixture".into(),
            family: "whisper".into(),
            backend: "fake".into(),
            runtime: None,
        });
        press(&mut app, KeyCode::Char('b'));
        let origin = app.screen;
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(app.screen, Screen::Folder);
        let history = app.navigation_history.clone();
        press(&mut app, KeyCode::Char('?'));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.screen, Screen::Folder);
        assert_eq!(app.navigation_history, history);
        press(&mut app, KeyCode::Char('d'));
        app.directory_input.clear();
        app.directory_input_error = Some("retained until cancelled".into());
        for ch in "qwmtb1234?".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        assert_eq!(app.directory_input, "qwmtb1234?");
        assert_eq!(app.screen, Screen::DirectoryInput);
        assert!(!app.should_quit);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.screen, Screen::Folder);
        assert!(app.directory_input_error.is_none());
        assert_eq!(app.navigation_history, history);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.screen, origin);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.screen, Screen::Web);
    }
}

#[test]
fn recording_keys_remain_local_and_discard_returns_to_the_opener() {
    for connected in [false, true] {
        let mut app = crate::tui::app::tests::ready_app();
        app.voice.target = connected.then(|| "invalid URL".into());
        app.screen = if connected {
            Screen::ServerTests
        } else {
            Screen::Bench
        };
        let origin = app.screen;
        // Only the screen state is exercised: no microphone is opened.
        app.navigate_to(Screen::Recording);
        let history = app.navigation_history.clone();
        for ch in "wmtb".chars() {
            press(&mut app, KeyCode::Char(ch));
            assert_eq!(app.screen, Screen::Recording);
            assert_eq!(app.navigation_history, history);
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.screen, Screen::Recording);
        assert_eq!(app.navigation_history, history);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.screen, origin);
        assert!(app.navigation_history.is_empty());
        assert!(!app.should_quit);
    }
}

#[test]
fn loading_back_waits_for_the_request_and_expired_busy_pages_are_skipped() {
    let mut app = crate::tui::app::tests::test_app();
    app.screen = Screen::Models;
    app.current_request = Some(7);
    app.pending_model_id = Some("fixture".into());
    app.navigate_to(Screen::Loading);
    let history = app.navigation_history.clone();
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Loading);
    assert_eq!(app.current_request, Some(7));
    assert_eq!(app.navigation_history, history);
    press(&mut app, KeyCode::Char('t'));
    press(&mut app, KeyCode::Char('?'));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Telemetry);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Loading);
    app.current_request = None;
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Models);

    for busy_screen in [Screen::Loading, Screen::Processing, Screen::Recording] {
        let mut app = crate::tui::app::tests::test_app();
        app.screen = Screen::Models;
        app.navigate_to(busy_screen);
        press(&mut app, KeyCode::Char('?'));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.screen, Screen::Models);
        assert!(app.navigation_history.is_empty());
    }
}

#[test]
fn new_work_cannot_reopen_busy_pages_from_an_earlier_operation() {
    let mut app = crate::tui::app::tests::test_app();
    app.screen = Screen::Models;
    app.current_request = Some(1);
    app.pending_model_id = Some("fixture".into());
    app.navigate_to(Screen::Loading);
    press(&mut app, KeyCode::Char('w'));
    app.current_request = Some(2);
    app.pending_model_id = None;
    app.navigate_to(Screen::Processing);
    assert!(!app.navigation_history.contains(&Screen::Loading));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Web);
    assert_eq!(app.current_request, Some(2));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Models);

    app.navigate_to(Screen::Recording);
    app.navigate_to(Screen::Processing);
    assert!(!app.navigation_history.contains(&Screen::Recording));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Models);
}

#[test]
fn bench_back_during_work_is_navigation_not_request_cancellation() {
    let mut app = crate::tui::app::tests::ready_app();
    app.screen = Screen::Models;
    app.navigate_to(Screen::Bench);
    app.current_request = Some(7);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Models);
    assert_eq!(app.current_request, Some(7));
}

#[test]
fn chat_requires_both_selected_models_to_be_ready() {
    for (voice, reply, runtime_ready) in [
        (false, false, false),
        (true, false, true),
        (false, true, true),
        (true, true, false),
        (true, true, true),
    ] {
        let mut app = crate::tui::app::tests::ready_app();
        if !voice {
            app.active_model = None;
        }
        if !reply {
            app.config.server_reply_model = None;
        }
        if !runtime_ready {
            app.local_runtime = None;
        }
        app.screen = Screen::Models;
        let ready = voice && reply && runtime_ready;
        press(&mut app, KeyCode::Char('b'));
        assert_eq!(
            app.screen,
            if ready { Screen::Bench } else { Screen::Models }
        );
        assert_eq!(app.chat_ready(), ready);
        if !ready {
            assert!(app
                .error_message
                .as_deref()
                .unwrap()
                .contains("Select a voice model and a reply model"));
            press(&mut app, KeyCode::Esc);
            assert_eq!(app.screen, Screen::Models);
        }
    }
    let mut app = crate::tui::app::tests::ready_app();
    app.pending_model_id = Some("new-voice".into());
    app.screen = Screen::Models;
    press(&mut app, KeyCode::Char('b'));
    assert_eq!(app.screen, Screen::Models);
    app.pending_model_id = None;
    press(&mut app, KeyCode::Char('b'));
    assert_eq!(app.screen, Screen::Bench);
}

#[test]
fn web_inspection_target_does_not_disable_local_model_loading() {
    let options = TuiOptions {
        server_url: Some("invalid URL".into()),
        model_id: Some("whisper".into()),
        model_manifest: None,
        audio_directory: None,
        max_seconds: None,
        language: None,
        dictionary: None,
        reconfigure: false,
    };
    let (sender, commands) = mpsc::channel::<WorkerCommand>();
    let (_, events) = mpsc::channel();
    let (_, metrics) = mpsc::channel();
    let mut app = App::new(
        &options,
        TuiConfig::default(),
        false,
        ModelCatalog {
            manifest_path: "fixture".into(),
            entries: vec![entry("whisper", "transcript", true)],
            error: None,
        },
        sender,
        events,
        metrics,
        Arc::new(AtomicU64::new(0)),
    );
    app.start_initial_load();
    assert!(matches!(
        commands.try_recv().unwrap(),
        WorkerCommand::LoadModel(_)
    ));
    assert!(app.current_request.is_some());
    assert!(app.voice.connection_error.is_some());
    assert!(
        app.chat.connection.is_none(),
        "Web target must not create a remote chat client"
    );
}

#[test]
fn editors_own_alphabet_and_digit_keys_and_submission_is_explicit() {
    let mut app = crate::tui::app::tests::ready_app();
    app.screen = Screen::Web;
    app.voice.target = Some("invalid URL".into());
    app.voice
        .inspect(Ok(crate::tui::voice::tests::snapshot("web", "partial")));
    press(&mut app, KeyCode::Char('e'));
    assert_eq!(app.screen, Screen::WebEditor);
    for ch in "qwmtb1234".chars() {
        press(&mut app, KeyCode::Char(ch));
    }
    app.handle_terminal_event(Event::Paste("界🙂".into()))
        .unwrap();
    assert_eq!(app.screen, Screen::WebEditor);
    assert_eq!(
        app.voice.editor.as_ref().unwrap().buffer.text,
        "originalqwmtb1234界🙂"
    );
    assert!(!app.should_quit);
    press(&mut app, KeyCode::Enter);
    assert!(!app.voice.editor.as_ref().unwrap().pending);
    press(&mut app, KeyCode::Enter);
    assert!(app.voice.editor.as_ref().unwrap().error.is_some());
    press(&mut app, KeyCode::Esc);
    assert!(app.voice.editor.as_ref().unwrap().buffer.dirty);
    press(&mut app, KeyCode::Char('b'));
    press(&mut app, KeyCode::Char('s'));
    press(&mut app, KeyCode::Char('i'));
    for ch in "qwmtb1234".chars() {
        press(&mut app, KeyCode::Char(ch));
    }
    assert_eq!(app.screen, Screen::ServerTests);
    assert_eq!(app.voice.tests.buffer.text, "qwmtb1234");
    assert!(!app.should_quit);
    press(&mut app, KeyCode::Enter);
    assert!(app.voice.tests.request.is_none());
    assert!(app.voice.tests.error.is_some());
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('t'));
    press(&mut app, KeyCode::Char('3'));
    assert_eq!(app.screen, Screen::Telemetry);
    assert_eq!(app.telemetry_tab, TelemetryTab::Runs);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::ServerTests);
}

#[test]
fn enter_download_requires_confirmation_and_filter_focus_blocks_navigation() {
    let mut app = crate::tui::app::tests::test_app();
    app.screen = Screen::Web;
    app.catalog.entries = vec![entry("fixture", "transcript", false)];
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Enter);
    assert!(
        matches!(app.models.confirmation, Some(Confirmation::Download(ref id)) if id == "fixture")
    );
    assert!(app.download.is_none());
    for ch in "wmtb1234/q".chars() {
        press(&mut app, KeyCode::Char(ch));
        assert_eq!(app.screen, Screen::Models);
    }
    assert!(!app.should_quit);
    assert!(!app.models.filtering);
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('s'));
    assert!(app.models.verification.is_none());
    assert!(app.config.server_stt_model.is_none());
    press(&mut app, KeyCode::Char('/'));
    for ch in "qwmtb1234".chars() {
        press(&mut app, KeyCode::Char(ch));
    }
    assert_eq!(app.models.group().filter, "qwmtb1234");
    assert_eq!(app.screen, Screen::Models);
    assert!(!app.should_quit);
    press(&mut app, KeyCode::Esc);
    assert!(!app.models.filtering);
}

#[test]
fn unified_rows_sync_catalog_index_before_standalone_enter_loads() {
    let mut app = crate::tui::app::tests::test_app();
    let (sender, commands) = mpsc::channel();
    app.worker_sender = sender;
    app.catalog.entries = vec![
        entry("stt-a", "transcript", true),
        entry("stt-b", "transcript", true),
        entry("reply", "reply", true),
    ];
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Down);
    assert_eq!(app.catalog_index, 1);
    press(&mut app, KeyCode::Down);
    assert_eq!(app.catalog_index, 2);
    assert_eq!(
        app.models.selected(&app.catalog).unwrap().manifest.id,
        "reply"
    );
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::Models);
    assert!(commands.try_recv().is_err());
    assert!(app.models.verification.is_none());
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::Models);
    assert!(
        matches!(commands.try_recv().unwrap(), WorkerCommand::LoadModel(request) if request.model_id == "stt-b")
    );
    assert!(app.active_model.is_none());
    assert!(app.current_request.is_some());
}

#[test]
fn catalog_filter_crosses_groups_and_syncs_the_selected_index() {
    let mut app = crate::tui::app::tests::test_app();
    app.catalog.entries = vec![
        entry("stt", "transcript", true),
        entry("reply", "reply", true),
    ];
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Char('/'));
    for ch in "reply".chars() {
        press(&mut app, KeyCode::Char(ch));
    }
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.catalog_index, 1);
    assert_eq!(app.models.rows(&app.catalog).len(), 1);
    press(&mut app, KeyCode::PageDown);
    assert_eq!(app.models.scroll, 5);
    press(&mut app, KeyCode::PageUp);
    assert_eq!(app.models.scroll, 0);
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Right);
    assert_eq!(app.catalog_index, 1);
}

#[test]
fn recording_and_other_focused_inputs_do_not_accept_navigation_shortcuts() {
    let mut app = crate::tui::app::tests::test_app();
    app.screen = Screen::Recording;
    for ch in "wmtb1234".chars() {
        press(&mut app, KeyCode::Char(ch));
        assert_eq!(app.screen, Screen::Recording);
    }
    app.screen = Screen::Telemetry;
    app.filter_editing = true;
    for ch in "wmtb1234".chars() {
        press(&mut app, KeyCode::Char(ch));
    }
    assert_eq!(app.filter_query, "wmtb1234");
    assert_eq!(app.screen, Screen::Telemetry);
    press(&mut app, KeyCode::Esc);
    app.clear_runs_pending = true;
    for ch in "wmtb".chars() {
        press(&mut app, KeyCode::Char(ch));
        assert_eq!(app.screen, Screen::Telemetry);
    }
    press(&mut app, KeyCode::Esc);
    app.screen = Screen::DirectoryInput;
    app.directory_input.clear();
    for ch in "wmtb1234".chars() {
        press(&mut app, KeyCode::Char(ch));
    }
    assert_eq!(app.screen, Screen::DirectoryInput);
    assert_eq!(app.directory_input, "wmtb1234");
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("pheme-workspace-{}-{suffix}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(std::fs::canonicalize(path).unwrap())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn role_picker_owns_navigation_filter_keys_and_arbitrary_path_input() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.0.join("incident-reporting.txt"),
        "Trusted incident role",
    )
    .unwrap();
    let mut app = crate::tui::app::tests::ready_app();
    let mut reply = entry("reply", "reply", true);
    reply.manifest.system_prompt = Some("incident-reporting.txt".into());
    app.catalog.manifest_path = fixture.0.join("manifest.toml");
    app.catalog.entries = vec![reply];
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Char('o'));
    assert!(app.models.role_selection.is_some());
    for ch in "wmtb1234/q".chars() {
        press(&mut app, KeyCode::Char(ch));
    }
    assert_eq!(app.screen, Screen::Models);
    assert!(!app.models.filtering);
    assert!(!app.should_quit);
    press(&mut app, KeyCode::Char('a'));
    for ch in "wmtb1234/q".chars() {
        press(&mut app, KeyCode::Char(ch));
    }
    app.handle_terminal_event(Event::Paste(" local path.txt".into()))
        .unwrap();
    assert_eq!(
        app.models.role_selection.as_ref().unwrap().path_input,
        "wmtb1234/q local path.txt"
    );
    press(&mut app, KeyCode::Enter);
    assert!(app.models.role_selection.as_ref().unwrap().error.is_some());
    assert!(app.config.reply_role_files.is_empty());
    press(&mut app, KeyCode::Esc);
    assert!(app.models.role_selection.is_some());
    press(&mut app, KeyCode::Esc);
    assert!(app.models.role_selection.is_none());
    press(&mut app, KeyCode::Char('p'));
    assert_eq!(
        app.models.preview.as_ref().unwrap().text,
        "Trusted incident role"
    );
    press(&mut app, KeyCode::Char('w'));
    assert_eq!(app.screen, Screen::Models);
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('g'));
    press(&mut app, KeyCode::Char('b'));
    assert_eq!(app.screen, Screen::Models);
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('b'));
    assert_eq!(app.screen, Screen::Bench);
}

#[test]
fn selecting_reply_is_remembered_automatically_and_prepared_for_activation() {
    use sha2::{Digest, Sha256};

    let fixture = Fixture::new();
    let role = fixture.0.join("incident-reporting.txt");
    std::fs::write(&role, "Trusted incident role").unwrap();
    std::fs::write(fixture.0.join("reply.gguf"), b"fixture").unwrap();
    let manifest = fixture.0.join("manifest.toml");
    std::fs::write(&manifest, format!(
        "[[models]]\nid='reply'\nfamily='qwen2'\npurpose='reply'\nruntime='llama.cpp'\nmodel='reply.gguf'\nsystem_prompt='incident-reporting.txt'\nsha256='{:x}'",
        Sha256::digest(b"fixture"),
    )).unwrap();
    let mut app = crate::tui::app::tests::test_app();
    app.catalog = ModelCatalog::load(manifest);
    app.config_path = fixture.0.join("tui.toml");
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Enter);
    assert!(app.models.verification.is_some());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while app.models.verification.is_some() && std::time::Instant::now() < deadline {
        app.tick_workspace();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(app.error_message.is_none(), "{:?}", app.error_message);
    assert!(app.models.confirmation.is_none());
    assert!(!app.models.command);
    assert_eq!(app.config.server_reply_model.as_deref(), Some("reply"));
    let saved: TuiConfig =
        toml::from_str(&std::fs::read_to_string(&app.config_path).unwrap()).unwrap();
    assert_eq!(saved.server_reply_model.as_deref(), Some("reply"));
    assert!(app.model_selected(&app.catalog.entries[0]));
    assert!(
        app.restart.is_some(),
        "new reply selection is activated automatically"
    );
    assert!(app.should_quit);
    assert!(app.current_request.is_none());
    assert!(app.active_model.is_none());
}

#[test]
fn reply_selection_failure_retains_previous_choice_and_never_opens_confirmation() {
    use sha2::{Digest, Sha256};
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("role.txt"), "Incident role").unwrap();
    std::fs::write(fixture.0.join("reply.gguf"), b"corrupt fixture").unwrap();
    let manifest = fixture.0.join("manifest.toml");
    std::fs::write(&manifest, format!("[[models]]\nid='reply'\nfamily='qwen2'\npurpose='reply'\nruntime='llama.cpp'\nmodel='reply.gguf'\nsystem_prompt='role.txt'\nsha256='{:x}'", Sha256::digest(b"good fixture"))).unwrap();
    let mut app = crate::tui::app::tests::test_app();
    app.catalog = ModelCatalog::load(manifest);
    app.config_path = fixture.0.join("tui.toml");
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Enter);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while app.models.verification.is_some() && std::time::Instant::now() < deadline {
        app.tick_workspace();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(app
        .error_message
        .as_deref()
        .is_some_and(|s| s.contains("checksum mismatch")));
    assert!(app.config.server_reply_model.is_none());
    assert!(!app.config_path.exists());
    assert!(app.models.confirmation.is_none());
    assert!(app.restart.is_none());
    assert!(!app.should_quit);
}

#[test]
fn selecting_the_active_reply_needs_no_verification_confirmation_or_restart() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("role.txt"), "Incident role").unwrap();
    std::fs::write(fixture.0.join("reply.gguf"), b"fixture").unwrap();
    let manifest = fixture.0.join("manifest.toml");
    std::fs::write(&manifest, "[[models]]\nid='fixture-reply'\nfamily='qwen2'\npurpose='reply'\nruntime='llama.cpp'\nmodel='reply.gguf'\nsystem_prompt='role.txt'").unwrap();
    let mut app = crate::tui::app::tests::ready_app();
    app.catalog = ModelCatalog::load(manifest);
    app.config_path = fixture.0.join("tui.toml");
    app.chat.editor.text = "Keep my draft".into();
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Enter);
    assert!(app.error_message.is_none(), "{:?}", app.error_message);
    assert!(app.models.verification.is_none());
    assert!(app.models.confirmation.is_none());
    assert!(app.restart.is_none());
    assert!(!app.should_quit);
    assert_eq!(app.chat.editor.text, "Keep my draft");
    let saved: TuiConfig =
        toml::from_str(&std::fs::read_to_string(&app.config_path).unwrap()).unwrap();
    assert_eq!(saved.server_reply_model.as_deref(), Some("fixture-reply"));
}

#[test]
fn failed_automatic_save_preserves_the_reply_selection_and_does_not_restart() {
    use sha2::{Digest, Sha256};
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("role.txt"), "Incident role").unwrap();
    std::fs::write(fixture.0.join("reply.gguf"), b"fixture").unwrap();
    let manifest = fixture.0.join("manifest.toml");
    std::fs::write(&manifest, format!("[[models]]\nid='reply'\nfamily='qwen2'\npurpose='reply'\nruntime='llama.cpp'\nmodel='reply.gguf'\nsystem_prompt='role.txt'\nsha256='{:x}'", Sha256::digest(b"fixture"))).unwrap();
    let mut app = crate::tui::app::tests::test_app();
    app.catalog = ModelCatalog::load(manifest);
    app.config.server_reply_model = Some("previous-reply".into());
    let blocked_parent = fixture.0.join("file-not-directory");
    std::fs::write(&blocked_parent, "original").unwrap();
    app.config_path = blocked_parent.join("tui.toml");
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Enter);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while app.models.verification.is_some() && std::time::Instant::now() < deadline {
        app.tick_workspace();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(app
        .error_message
        .as_deref()
        .unwrap()
        .contains("Could not select reply"));
    assert_eq!(
        app.config.server_reply_model.as_deref(),
        Some("previous-reply")
    );
    assert_eq!(std::fs::read_to_string(blocked_parent).unwrap(), "original");
    assert!(app.restart.is_none());
    assert!(!app.should_quit);
    assert!(app.models.confirmation.is_none());
}

#[test]
fn failed_voice_switch_keeps_the_selected_pair_available_and_can_retry() {
    let mut app = crate::tui::app::tests::ready_app();
    app.catalog.entries = vec![entry("voice-b", "transcript", true)];
    let (sender, commands) = mpsc::channel();
    app.worker_sender = sender;
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Enter);
    let WorkerCommand::LoadModel(request) = commands.try_recv().unwrap() else {
        panic!("model load expected");
    };
    app.apply_worker_event(crate::tui::events::WorkerEvent::ModelLoadFailed {
        request_id: request.request_id,
        model_id: request.model_id,
        error: "fixture failure".into(),
        switch_from: request.switch_from,
    });
    assert_eq!(app.config.selected_stt_model, "fixture-stt");
    assert_eq!(app.active_model.as_ref().unwrap().id, "fixture-stt");
    assert!(
        app.chat_ready(),
        "failed switch must not leave a preparation gate active"
    );
    press(&mut app, KeyCode::Char('r'));
    assert!(
        matches!(commands.try_recv().unwrap(), WorkerCommand::LoadModel(request) if request.model_id == "voice-b")
    );
    assert!(!app.chat_ready());
}

#[test]
fn reply_startup_failure_remains_visible_after_voice_becomes_ready() {
    let fixture = Fixture::new();
    let mut app = crate::tui::app::tests::test_app();
    app.config_path = fixture.0.join("tui.toml");
    app.screen = Screen::Models;
    app.config.server_reply_model = Some("reply".into());
    let runtime = va_runtime::AgentRuntime::new(
        None,
        None,
        None,
        va_runtime::Limits::default(),
        metrics::MetricsConfig::enabled(),
        Arc::new(metrics::MetricsHub::new()),
    );
    app.apply_worker_event(crate::tui::events::WorkerEvent::RuntimeReady {
        runtime,
        reply_error: Some("reply worker missing".into()),
    });
    app.current_request = Some(1);
    app.apply_worker_event(crate::tui::events::WorkerEvent::ModelReady {
        request_id: 1,
        model_id: "voice".into(),
        model_family: "whisper".into(),
        backend: "fake".into(),
        load_duration_ms: 1,
        switch_from: None,
    });
    assert!(app
        .error_message
        .as_deref()
        .unwrap()
        .contains("reply worker missing"));
    assert!(!app.chat_ready());
    assert_eq!(app.screen, Screen::Models);
    app.shutdown_workspace();
}

#[test]
fn model_preparation_cannot_restart_and_discard_an_unsent_chat_draft() {
    let mut app = crate::tui::app::tests::ready_app();
    let mut voice = entry("uncompiled-voice", "transcript", true);
    voice.adapter_compiled = false;
    app.catalog.entries = vec![voice];
    app.chat.editor.text = "Keep my draft".into();
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Enter);
    assert!(app
        .error_message
        .as_deref()
        .unwrap()
        .contains("finish the pending turn"));
    assert!(app.build.is_none());
    assert!(app.restart.is_none());
    assert_eq!(app.config.selected_stt_model, "fixture-stt");
    assert_eq!(app.chat.editor.text, "Keep my draft");
}

#[test]
fn voice_selection_is_saved_and_restored_without_reselecting_models() {
    use crate::tui::events::WorkerEvent;
    let fixture = Fixture::new();
    let mut app = crate::tui::app::tests::ready_app();
    app.config_path = fixture.0.join("tui.toml");
    app.catalog.entries = vec![
        entry("voice-a", "transcript", true),
        entry("voice-b", "transcript", true),
        entry("fixture-reply", "reply", true),
    ];
    let (sender, commands) = mpsc::channel();
    app.worker_sender = sender;
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    let WorkerCommand::LoadModel(request) = commands.try_recv().unwrap() else {
        panic!("selection must load the highlighted voice model");
    };
    assert_eq!(request.model_id, "voice-b");
    app.apply_worker_event(WorkerEvent::ModelReady {
        request_id: request.request_id,
        model_id: request.model_id,
        model_family: "whisper".into(),
        backend: "fake".into(),
        load_duration_ms: 1,
        switch_from: request.switch_from,
    });
    assert!(app.error_message.is_none());
    assert!(app.chat_ready());
    assert!(app.model_selected(&app.catalog.entries[1]));
    assert!(app.model_selected(&app.catalog.entries[2]));
    let saved: TuiConfig =
        toml::from_str(&std::fs::read_to_string(&app.config_path).unwrap()).unwrap();
    let options = TuiOptions {
        server_url: None,
        model_id: None,
        model_manifest: None,
        audio_directory: None,
        max_seconds: None,
        language: None,
        dictionary: None,
        reconfigure: false,
    };
    let (sender, commands) = mpsc::channel();
    let (_, events) = mpsc::channel();
    let (_, metrics) = mpsc::channel();
    let mut restored = App::new(
        &options,
        saved,
        true,
        app.catalog.clone(),
        sender,
        events,
        metrics,
        Arc::new(AtomicU64::new(0)),
    );
    assert_eq!(restored.screen, Screen::Models);
    assert_eq!(restored.config.selected_stt_model, "voice-b");
    assert_eq!(
        restored.config.server_reply_model.as_deref(),
        Some("fixture-reply")
    );
    assert!(restored.model_selected(&restored.catalog.entries[1]));
    assert!(restored.model_selected(&restored.catalog.entries[2]));
    assert!(
        !restored.chat_ready(),
        "wait for the restored models to load"
    );
    restored.start_initial_load();
    assert!(
        matches!(commands.try_recv().unwrap(), WorkerCommand::LoadModel(request) if request.model_id == "voice-b")
    );
    assert_eq!(restored.screen, Screen::Models);
}

#[test]
fn fake_local_download_remains_inline_and_completes_without_model_activation() {
    let fixture = Fixture::new();
    let manifest = fixture.0.join("manifest.toml");
    std::fs::write(
        &manifest,
        "[[models]]\nid='fixture'\nfamily='whisper'\nmodel='fixture.bin'",
    )
    .unwrap();
    std::fs::write(fixture.0.join("fixture.bin"), b"small local fixture").unwrap();
    let script = fixture.0.join("fake-download.sh");
    std::fs::write(&script, "printf '100%% fixture complete\\n'\nexit 0\n").unwrap();
    let mut app = crate::tui::app::tests::test_app();
    app.catalog = ModelCatalog::load(manifest);
    app.screen = Screen::Models;
    app.download =
        Some(DownloadTask::start_script("fixture".into(), "fixture", &fixture.0, &script).unwrap());
    press(&mut app, KeyCode::PageDown);
    assert_eq!(app.screen, Screen::Models);
    assert!(app.download.is_some());
    press(&mut app, KeyCode::Char('t'));
    assert_eq!(app.screen, Screen::Telemetry);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Models);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while app.download.is_some() && std::time::Instant::now() < deadline {
        app.poll_download();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(app.download.is_none());
    assert_eq!(app.screen, Screen::Models);
    assert_eq!(
        app.models.download_status["fixture"],
        "Downloaded (script verified)"
    );
    assert!(app.active_model.is_none());
    assert!(app.build.is_none());
    assert!(app.current_request.is_none());
    assert!(app.config.server_stt_model.is_none());
}
