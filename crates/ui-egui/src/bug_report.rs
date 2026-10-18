//! Help › Report an Issue…: a bug report with diagnostics the user reviews before anything
//! leaves the app, opened as a prefilled GitHub issue in the browser.
//!
//! The dialog shows exactly the text that will be sent. Diagnostics are opt-out by section
//! (system, open documents) or opt-in (recent notices, which can name files); the user's home
//! folder is replaced by `~` everywhere, and no file names, paths or pixels are collected.
//! Nothing is uploaded: the report travels in the URL of GitHub's new-issue page, where the user
//! can still edit it before submitting. Automation drives the same path with
//! `help.reportIssue {"title","description","include":{…},"open":bool}`.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

const MARK: &str = "__bugReport";
pub const NEW_ISSUE: &str = "https://github.com/treeforjs/photocraft/issues/new";
/// GitHub refuses new-issue URLs much past 8 KB; the body is cut to keep under this.
pub const MAX_URL: usize = 7500;
const MAX_TITLE: usize = 200;
const MAX_DESCRIPTION: usize = 4000;

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Which diagnostic sections go into the report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Include {
    pub system: bool,
    pub documents: bool,
    pub notices: bool,
}

impl Default for Include {
    fn default() -> Self {
        Include { system: true, documents: true, notices: false }
    }
}

impl Include {
    fn from(v: Option<&Value>) -> Include {
        let d = Include::default();
        let flag = |k: &str, def: bool| v.and_then(|v| v.get(k)).and_then(Value::as_bool).unwrap_or(def);
        Include { system: flag("system", d.system), documents: flag("documents", d.documents), notices: flag("notices", d.notices) }
    }

    fn to_json(self) -> Value {
        json!({"system": self.system, "documents": self.documents, "notices": self.notices})
    }
}

/// The facts a report can carry, already redacted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diagnostics {
    pub system: Vec<String>,
    pub documents: Vec<String>,
    pub notices: Vec<String>,
}

pub fn diagnostics(app: &PhotocraftApp) -> Diagnostics {
    let home = home_dir();
    let r = |s: &str| redact(s, home.as_deref());
    let mut system: Vec<String> = crate::gpu_status::system_info(app).iter().map(|l| r(l)).collect();
    system.push(format!("UI language: {}", crate::i18n::current().code()));
    let documents = app
        .session
        .documents()
        .iter()
        .map(|st| {
            let d = &st.doc;
            format!("{}×{} px, {:?}, {:?}, {} layers", d.size.width, d.size.height, d.mode, d.depth, d.layer_count())
        })
        .collect();
    let notices = app.ui.notices.iter().rev().take(5).map(|n| r(&n.title)).collect();
    Diagnostics { system, documents, notices }
}

fn home_dir() -> Option<String> {
    ["HOME", "USERPROFILE"].iter().find_map(|k| std::env::var(k).ok()).filter(|h| h.len() > 1)
}

/// Replaces the home folder with `~`.
pub fn redact(s: &str, home: Option<&str>) -> String {
    match home {
        Some(h) if !h.is_empty() => s.replace(h, "~"),
        _ => s.to_string(),
    }
}

fn clip(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((i, _)) => s.get(..i).unwrap_or(s),
        None => s,
    }
}

/// The issue body (GitHub Markdown).
pub fn compose(description: &str, diag: &Diagnostics, include: Include) -> String {
    let mut out = String::new();
    let description = clip(description.trim(), MAX_DESCRIPTION);
    out.push_str("### What happened\n\n");
    out.push_str(if description.is_empty() { "_Describe what you did, what you expected and what happened instead._" } else { description });
    out.push('\n');
    let mut section = |title: &str, lines: &[String]| {
        if lines.is_empty() {
            return;
        }
        out.push_str(&format!("\n### {title}\n\n```\n"));
        for l in lines {
            // A line can't end the code block early.
            out.push_str(&l.replace("```", "'''"));
            out.push('\n');
        }
        out.push_str("```\n");
    };
    if include.system {
        section("System", &diag.system);
    }
    if include.documents {
        section("Open documents", &diag.documents);
    }
    if include.notices {
        section("Recent notices", &diag.notices);
    }
    out
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// GitHub's new-issue page prefilled with `title` and `body`, the body cut at a character
/// boundary (with a note) so the URL stays under [`MAX_URL`].
pub fn issue_url(title: &str, body: &str) -> String {
    let head = format!("{NEW_ISSUE}?labels=bug&title={}&body=", encode(clip(title.trim(), MAX_TITLE)));
    let full = encode(body);
    if head.len() + full.len() <= MAX_URL {
        return head + &full;
    }
    let note = encode("\n\n_(cut to fit the link; paste the rest here)_");
    let room = MAX_URL.saturating_sub(head.len() + note.len());
    let mut cut = String::new();
    for c in body.chars() {
        let e = encode(c.encode_utf8(&mut [0; 4]));
        if cut.len() + e.len() > room {
            break;
        }
        cut.push_str(&e);
    }
    head + &cut + &note
}

/// Opens the report dialog.
pub fn open(app: &mut PhotocraftApp) -> u64 {
    if let Some(d) = app.ui.dialogs.iter().find(|d| owns(&d.fields)) {
        return d.id;
    }
    let fields = json!({MARK: true, "__label": "Report an Issue", "title": "", "description": "", "include": Include::default().to_json()});
    app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap_or_default())
}

fn report(app: &PhotocraftApp, p: &Value) -> (String, String) {
    // An untitled report still opens; GitHub asks for a title before it can be submitted.
    let title = p.get("title").and_then(Value::as_str).unwrap_or("").to_string();
    let description = p.get("description").and_then(Value::as_str).unwrap_or("");
    let body = compose(description, &diagnostics(app), Include::from(p.get("include")));
    (title, body)
}

/// `help.reportIssue`: the dialog without params; with params the report itself, opened in
/// the browser only when `open` is true.
pub fn run(app: &mut PhotocraftApp, ctx: &egui::Context, p: &Value) -> Result<Value, String> {
    if p.as_object().is_none_or(|o| o.is_empty()) {
        return Ok(json!({"dialog": open(app)}));
    }
    let (title, body) = report(app, p);
    let url = issue_url(&title, &body);
    if p.get("open").and_then(Value::as_bool) == Some(true) {
        crate::links::open(app, ctx, &url);
    }
    Ok(json!({"title": title, "body": body, "url": url}))
}

pub fn body(app: &PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut title = fields.get("title").and_then(Value::as_str).unwrap_or("").to_string();
    let mut description = fields.get("description").and_then(Value::as_str).unwrap_or("").to_string();
    let mut include = Include::from(fields.get("include"));
    ui.label(tl!("Title"));
    let w = ui.available_width();
    ui.add(egui::TextEdit::singleline(&mut title).hint_text(tl!("A short summary of the problem")).desired_width(w).char_limit(MAX_TITLE));
    ui.add_space(6.0);
    ui.label(tl!("What happened?"));
    ui.add(
        egui::TextEdit::multiline(&mut description)
            .hint_text(tl!("Steps to reproduce, what you expected, what happened instead"))
            .desired_width(w)
            .desired_rows(5)
            .char_limit(MAX_DESCRIPTION),
    );
    ui.add_space(8.0);
    ui.label(egui::RichText::new(tl!("Include diagnostics")).font(crate::theme::semibold(13.0)));
    crate::widgets::checkbox(ui, &mut include.system, tl!("System (version, platform, GPU, displays)"));
    crate::widgets::checkbox(ui, &mut include.documents, tl!("Open documents (size, mode, depth, layer count; no names)"));
    crate::widgets::checkbox(ui, &mut include.notices, tl!("Recent notices (may name files)"));
    ui.add_space(8.0);
    let p = json!({"title": title, "description": description, "include": include.to_json()});
    let (_, text) = report(app, &p);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("This is everything that will be sent:")).color(t.text_faint));
        if crate::widgets::secondary_button(ui, tl!("Copy"), 64.0).clicked() {
            ui.ctx().copy_text(text.clone());
        }
    });
    egui::ScrollArea::vertical().id_salt("bug-report-preview").max_height(180.0).show(ui, |ui| {
        ui.add(egui::Label::new(egui::RichText::new(&text).font(crate::theme::mono(11.5))).selectable(true).wrap());
    });
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("Opens a new issue on GitHub in your browser; you can still edit it there.")).color(t.text_faint));
    fields.insert("title".into(), json!(title));
    fields.insert("description".into(), json!(description));
    fields.insert("include".into(), include.to_json());
}

pub fn ok_label(fields: &Map<String, Value>) -> Option<&'static str> {
    owns(fields).then(|| tl!("Open on GitHub"))
}

/// OK: the browser opens on the next frame (dialogs confirm without the egui context).
pub fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let p = Value::Object(fields.clone());
    let (title, body) = report(app, &p);
    let url = issue_url(&title, &body);
    app.ui.pending_url = Some(url.clone());
    Ok(json!({"url": url}))
}

#[cfg(test)]
#[path = "bug_report_tests.rs"]
mod tests;
