use super::*;

fn app_with_doc() -> PhotocraftApp {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 320, "height": 200})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Secret plans"})).unwrap();
    PhotocraftApp::new(s, crate::Services::default())
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap(), 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

fn query<'a>(url: &'a str, key: &str) -> &'a str {
    url.split(['?', '&']).find_map(|kv| kv.strip_prefix(&format!("{key}="))).unwrap()
}

#[test]
fn the_report_carries_what_the_dialog_shows_and_no_names() {
    let mut app = app_with_doc();
    let ctx = egui::Context::default();
    let r = crate::menus::invoke(&mut app, &ctx, "help.reportIssue", json!({"title": "Brush lags", "description": "Paint fast & see lag"})).unwrap();
    let body = r["body"].as_str().unwrap();
    assert!(body.contains("Paint fast & see lag"));
    assert!(body.contains("### System") && body.contains("PhotoCraft "), "{body}");
    assert!(body.contains("320×200 px") && body.contains("2 layers"), "{body}");
    assert!(!body.contains("Secret plans") && !body.contains("Untitled"), "no layer or document names: {body}");
    let url = r["url"].as_str().unwrap();
    assert!(url.starts_with(NEW_ISSUE));
    assert_eq!(decode(query(url, "title")), "Brush lags");
    assert_eq!(decode(query(url, "body")), body, "the URL carries exactly the previewed text");
    assert!(app.ui.status.is_empty() || !app.ui.status.starts_with("Opened"), "nothing opens without `open`");
}

#[test]
fn sections_can_be_left_out() {
    let mut app = app_with_doc();
    let ctx = egui::Context::default();
    let p = json!({"title": "x", "include": {"system": false, "documents": false}});
    let body = crate::menus::invoke(&mut app, &ctx, "help.reportIssue", p).unwrap()["body"].as_str().unwrap().to_string();
    assert!(!body.contains("### System") && !body.contains("### Open documents"), "{body}");
    // Notices are opt-in.
    crate::notices::post(&mut app, "Couldn't open ~/x.psd", vec![], true, None);
    let body = compose("", &diagnostics(&app), Include::default());
    assert!(!body.contains("Recent notices"));
    let body = compose("", &diagnostics(&app), Include { notices: true, ..Include::default() });
    assert!(body.contains("### Recent notices") && body.contains("Couldn't open"), "{body}");
}

#[test]
fn the_home_folder_is_redacted() {
    assert_eq!(redact("/home/ana/art/cat.psd failed", Some("/home/ana")), "~/art/cat.psd failed");
    assert_eq!(redact("C:\\Users\\Ana\\x", Some("C:\\Users\\Ana")), "~\\x");
    assert_eq!(redact("nothing here", None), "nothing here");
    assert_eq!(redact("keep /", Some("")), "keep /");
}

#[test]
fn a_long_report_is_cut_to_fit_the_link_at_a_character_boundary() {
    let body = "é✓ ".repeat(5000);
    let url = issue_url("Long", &body);
    assert!(url.len() <= MAX_URL, "{}", url.len());
    let sent = decode(query(&url, "body"));
    assert!(sent.ends_with("_(cut to fit the link; paste the rest here)_"), "{}", &sent[sent.len() - 80..]);
    assert!(body.starts_with(sent.split("\n\n_(cut").next().unwrap()));
    // A short one is sent whole, and hostile text can't break out of the query.
    let url = issue_url("a&b=c?#", "```\nx\n```");
    assert_eq!(decode(query(&url, "title")), "a&b=c?#");
    assert!(!url.contains('#') && url.matches('&').count() == 2, "{url}");
}

#[test]
fn a_code_fence_in_diagnostics_cannot_close_the_block() {
    let diag = Diagnostics { system: vec!["GPU ```evil".into()], ..Diagnostics::default() };
    let body = compose("hi", &diag, Include::default());
    assert_eq!(body.matches("```").count(), 2, "{body}");
}

#[test]
fn the_dialog_opens_once_and_ok_opens_the_browser() {
    let opened = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink = opened.clone();
    let services = crate::Services {
        open_url: Some(Box::new(move |u: &str| {
            sink.lock().unwrap().push(u.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let mut app = PhotocraftApp::new(s, services);
    let ctx = egui::Context::default();
    let a = crate::menus::invoke(&mut app, &ctx, "help.reportIssue", json!({})).unwrap()["dialog"].as_u64().unwrap();
    let b = crate::menus::invoke(&mut app, &ctx, "help.reportIssue", json!({})).unwrap()["dialog"].as_u64().unwrap();
    assert_eq!(a, b, "a second click focuses the open report");
    let d = app.ui.dialogs.iter_mut().find(|d| d.id == a).unwrap();
    d.fields.insert("title".into(), json!("Crash on save"));
    let r = crate::dialogs::confirm(&mut app, a).unwrap();
    assert!(app.ui.dialogs.is_empty());
    assert!(opened.lock().unwrap().is_empty(), "opens on the next frame");
    let mut h = egui_kittest::Harness::builder().build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Craft);
    h.run();
    let got = opened.lock().unwrap().clone();
    assert_eq!(got, [r["url"].as_str().unwrap().to_string()]);
    assert_eq!(decode(query(&got[0], "title")), "Crash on save");
    assert!(h.state().ui.pending_url.is_none());
}

#[test]
fn the_dialog_renders_its_preview() {
    use egui_kittest::kittest::Queryable;
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(900.0, 800.0))
        .build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app_with_doc());
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Craft);
    open(h.state_mut());
    h.run();
    h.get_by_label("Open on GitHub");
    assert!(h.query_by_label_contains("320×200 px").is_some(), "the preview lists the open document");
}
