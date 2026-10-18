//! `cargo xtask ui-commands`: the UI changes documents only through command ids.
//!
//! Every panel button, menu item and tool ends in `app.run(id, params)` (or `session.execute`),
//! so the control channel, MCP, actions and history see the same edit. This check scans the
//! non-test code of `crates/ui-egui/src` for the ways a widget could bypass that: editing the
//! live document (`session.edit`, `Arc::make_mut(&mut st.doc)`, assigning `st.doc`), rewriting
//! history, or moving `revision` by hand. Editing a preview copy is fine and isn't matched.
//! Each remaining exception is listed in [`ALLOWED`] with the reason it can't be a command.

use std::fs;
use std::path::{Path, PathBuf};

/// Source text that bypasses the command system. `st` and `state` are the names the UI gives a
/// live `DocState` (from `session.active_mut()`); a pattern starting with a name matches only as
/// a whole word, so `first.revision` or a preview's `pv.doc` don't count.
const FORBIDDEN: &[(&str, &str)] = &[
    (".session.edit(", "edits the document without a command"),
    ("make_mut(&mut st.doc)", "edits the live document without a command"),
    ("make_mut(&mut state.doc)", "edits the live document without a command"),
    ("st.doc = ", "replaces the live document without a command"),
    ("state.doc = ", "replaces the live document without a command"),
    (".history.push", "adds a history step without a command"),
    (".history.purge_last(", "rewrites history without a command"),
    (".history.set_current_label(", "renames a history step without a command"),
    (".history.clear(", "clears history without a command"),
    ("st.revision += ", "marks the document changed without a command"),
    ("st.revision = ", "marks the document changed without a command"),
    ("state.revision = ", "marks the document changed without a command"),
    ("st.saved_revision = ", "marks the document saved without a command"),
    ("state.saved_revision = ", "marks the document saved without a command"),
];

/// Whether `pattern` occurs in `code`, as a whole word when it starts with one.
fn contains(code: &str, pattern: &str) -> bool {
    let word = pattern.starts_with(|c: char| c.is_ascii_alphabetic());
    code.match_indices(pattern)
        .any(|(i, _)| !word || !code.get(..i).and_then(|s| s.chars().next_back()).is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.'))
}

/// `(file under crates/ui-egui/src, forbidden text, why it isn't a command)`.
const ALLOWED: &[(&str, &str, &str)] = &[
    ("move_mods.rs", ".history.purge_last(", "Alt-drag duplicate + move folds the duplicate and the move commands it just ran into one undo step"),
    ("move_mods.rs", ".history.set_current_label(", "names the folded Duplicate + Move step"),
    ("prefs_ui.rs", "st.saved_revision = ", "a recovered autosave opens inside the open job as unsaved, so closing it asks to save"),
    ("lib.rs", "st.saved_revision = ", "a finished background save records the revision it wrote"),
    ("camera_raw_ui.rs", "st.saved_revision = ", "a raw file just developed into a document has nothing to save yet"),
    ("transform_tool.rs", ".history.purge_last(", "the step that made a layer (copy, Place Embedded) and the transform that follows become one undo step"),
    ("transform_tool.rs", ".history.set_current_label(", "names that folded step"),
];

pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("crates/ui-egui/src");
    let mut files = Vec::new();
    collect(&dir, &mut files)?;
    files.sort();
    let mut problems = Vec::new();
    let mut used = vec![false; ALLOWED.len()];
    for path in &files {
        let rel = path.strip_prefix(&dir).unwrap_or(path).to_string_lossy().replace('\\', "/");
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        for (line, pattern, why) in scan(&text) {
            match ALLOWED.iter().position(|(f, p, _)| *f == rel && *p == pattern) {
                Some(i) => {
                    if let Some(u) = used.get_mut(i) {
                        *u = true;
                    }
                }
                None => problems.push(format!("crates/ui-egui/src/{rel}:{line}: `{pattern}` {why}; run a command id instead")),
            }
        }
    }
    for ((file, pattern, _), used) in ALLOWED.iter().zip(&used) {
        if !used {
            problems.push(format!("xtask/src/ui_commands.rs: the exception for `{pattern}` in {file} is no longer needed; remove it"));
        }
    }
    if problems.is_empty() {
        println!("ui-commands: {} UI files change documents only through command ids ({} listed exceptions)", files.len(), ALLOWED.len());
        Ok(())
    } else {
        for p in &problems {
            eprintln!("{p}");
        }
        Err(format!("{} UI edit(s) bypass the command system", problems.len()))
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if name != "tests" {
                collect(&path, out)?;
            }
        } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// `(line number, forbidden text, why)` for each match outside `#[cfg(test)]` items and comments.
fn scan(text: &str) -> Vec<(usize, &'static str, &'static str)> {
    let mut hits = Vec::new();
    // Inside a `#[cfg(test)]` item: the brace depth at which it ends, once its body opened.
    let mut skip: Option<i64> = None;
    let mut pending_test = false;
    let mut depth: i64 = 0;
    for (i, line) in text.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        let trimmed = code.trim();
        let start = depth;
        depth += code.matches('{').count() as i64 - code.matches('}').count() as i64;
        if skip.is_some() {
            if skip.is_some_and(|end| depth <= end) {
                skip = None;
            }
            continue;
        }
        if trimmed.starts_with("#[cfg(test)]") {
            pending_test = true;
            continue;
        }
        if pending_test {
            if trimmed.is_empty() || trimmed.starts_with("#[") {
                continue;
            }
            pending_test = false;
            // A one-line item (`mod tests;`, `use …;`) ends here; a block runs until it closes.
            if depth > start {
                skip = Some(start);
            }
            continue;
        }
        for (pattern, why) in FORBIDDEN {
            if contains(code, pattern) {
                hits.push((i + 1, *pattern, *why));
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_bypasses_outside_tests_and_comments() {
        let src = "fn ok(app: &mut App) { app.run(\"edit.fill\", json!({})); }\n\
                   fn bad(st: &mut DocState) {\n    Arc::make_mut(&mut st.doc).layers.clear();\n    st.revision += 1;\n}\n\
                   // st.revision += 1 in a comment\n\
                   fn preview(p: &mut Document) { p.layer_mut(id); }\n\
                   #[cfg(test)]\nmod tests {\n    fn t() {\n        st.revision += 1;\n        s.session.edit(\"x\", |_, _| Ok(()));\n    }\n}\n\
                   fn after() { app.session.edit(\"x\", f); }\n";
        let hits: Vec<_> = scan(src).into_iter().map(|(l, p, _)| (l, p)).collect();
        assert_eq!(hits, [(3, "make_mut(&mut st.doc)"), (4, "st.revision += "), (15, ".session.edit(")]);
    }

    #[test]
    fn a_one_line_test_item_skips_only_itself() {
        let src = "#[cfg(test)]\n#[path = \"x_tests.rs\"]\nmod tests;\nfn f() { st.saved_revision = 0; }\n";
        assert_eq!(scan(src).len(), 1);
    }

    #[test]
    fn names_match_whole_words_only() {
        assert!(contains("    st.revision = 3;", "st.revision = "));
        assert!(!contains("    first.revision = 3;", "st.revision = "));
        assert!(!contains("    self.st.revision = 3;", "st.revision = "));
        assert!(contains("x.history.purge_last()", ".history.purge_last("));
    }

    #[test]
    fn the_ui_crate_passes() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        run(&root).unwrap();
    }
}
